use anyhow::Context;
use chrono::{DateTime, Utc};
use entities::tasks;
use rand::Rng;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, Database, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
    sea_query::{LockBehavior, LockType},
};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("task_worker=info")),
        )
        .init();

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable is required");

    // Pool connections up front so the worker can sprint once tasks arrive.
    let db = Database::connect(&database_url)
        .await
        .expect("failed to connect to database");

    info!("task worker started");

    run_worker(db).await;
}

async fn run_worker(db: DatabaseConnection) {
    loop {
        match fetch_and_tag_next_task(&db).await {
            Ok(Some(task)) => {
                if let Err(err) = process_task(&db, task).await {
                    error!(error = %err, "task processing failed");
                }
            }
            Ok(None) => {
                sleep(Duration::from_secs(1)).await;
            }
            Err(err) => {
                error!(error = %err, "failed to fetch next task");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn fetch_and_tag_next_task(db: &DatabaseConnection) -> Result<Option<tasks::Model>, DbErr> {
    let now = Utc::now();

    let txn = db.begin().await?;

    // Transaction + SKIP LOCKED prevents double picks once we scale horizontally.
    let pending = tasks::Entity::find()
        .filter(tasks::Column::State.eq(tasks::TaskState::Pending))
        .filter(tasks::Column::ExecutionTime.lte(now))
        .order_by_asc(tasks::Column::ExecutionTime)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .one(&txn)
        .await?;

    let Some(task) = pending else {
        txn.rollback().await?;
        return Ok(None);
    };

    let mut active: tasks::ActiveModel = task.into();
    active.state = Set(tasks::TaskState::Running);

    let started = active.update(&txn).await?;

    txn.commit().await?;

    info!(
        task_id = %started.id,
        task_type = started.task_type,
        execution_time = %fmt_time(started.execution_time),
        "task marked as running"
    );

    Ok(Some(started))
}

async fn process_task(db: &DatabaseConnection, task: tasks::Model) -> Result<(), DbErr> {
    info!(
        task_id = %task.id,
        task_type = task.task_type,
        "processing task"
    );

    let task_id = task.id;
    // Execute outside the transaction so slow jobs don't block the queue.
    let result = execute_task(&task).await;

    match result {
        Ok(()) => {
            update_task_state(db, task, tasks::TaskState::Completed).await?;
            info!(task_id = %task_id, "task completed successfully");
        }
        Err(err) => {
            error!(task_id = %task_id, error = %err, "task execution failed, marking as failed");
            update_task_state(db, task, tasks::TaskState::Failed).await?;
        }
    }

    Ok(())
}

async fn update_task_state(
    db: &DatabaseConnection,
    task: tasks::Model,
    state: tasks::TaskState,
) -> Result<tasks::Model, DbErr> {
    let mut active: tasks::ActiveModel = task.into();
    active.state = Set(state);
    let updated = active.update(db).await?;

    info!(
        task_id = %updated.id,
        new_state = %format!("{state:?}").to_lowercase(),
        "task state updated"
    );

    Ok(updated)
}

async fn execute_task(task: &tasks::Model) -> anyhow::Result<()> {
    let task_type = TaskType::parse(&task.task_type)
        .ok_or_else(|| anyhow::anyhow!("unknown task type: {}", task.task_type))?;

    match task_type {
        TaskType::Foo => {
            sleep(Duration::from_secs(3)).await;
            println!("Foo {}", task.id);
        }
        TaskType::Bar => {
            let response = reqwest::Client::new()
                .get("https://www.whattimeisitrightnow.com/")
                .send()
                .await
                .with_context(|| "failed to execute Bar task")?;
            println!("Bar {}", response.status());
        }
        TaskType::Baz => {
            let mut rng = rand::thread_rng();
            let n: u16 = rng.gen_range(0..=343);
            println!("Baz {n}");
        }
    }

    Ok(())
}

fn fmt_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskType {
    Foo,
    Bar,
    Baz,
}

impl TaskType {
    fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "foo" => Some(TaskType::Foo),
            "bar" => Some(TaskType::Bar),
            "baz" => Some(TaskType::Baz),
            _ => None,
        }
    }
}

mod entities {
    pub mod tasks {
        use sea_orm::entity::prelude::*;
        use serde::{Deserialize, Serialize};

        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize,
        )]
        #[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "task_state")]
        #[serde(rename_all = "lowercase")]
        pub enum TaskState {
            #[sea_orm(string_value = "pending")]
            Pending,
            #[sea_orm(string_value = "running")]
            Running,
            #[sea_orm(string_value = "completed")]
            Completed,
            #[sea_orm(string_value = "failed")]
            Failed,
        }

        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "tasks")]
        pub struct Model {
            #[sea_orm(primary_key)]
            pub id: Uuid,
            pub task_type: String,
            pub execution_time: DateTimeUtc,
            pub state: TaskState,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }
}
