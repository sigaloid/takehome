mod migrator;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Database, DatabaseConnection, DbErr,
    EntityTrait, QueryFilter,
};
use sea_orm_migration::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use entities::tasks;
use migrator::Migrator;
use std::fmt;

#[derive(Clone)]
struct AppState {
    db: DatabaseConnection,
}

#[derive(Debug, Deserialize)]
struct CreateTaskRequest {
    task_type: String,
    execution_time: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct CreateTaskResponse {
    id: Uuid,
}

#[derive(Debug, Default, Deserialize)]
struct ListTasksQuery {
    state: Option<tasks::TaskState>,
    task_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Task {
    id: Uuid,
    task_type: String,
    execution_time: DateTime<Utc>,
    state: tasks::TaskState,
}

impl From<tasks::Model> for Task {
    fn from(model: tasks::Model) -> Self {
        Task {
            id: model.id,
            task_type: model.task_type,
            execution_time: model.execution_time,
            state: model.state,
        }
    }
}

#[tokio::main]
async fn main() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable is required");
    let db = Database::connect(&database_url)
        .await
        .expect("failed to connect to database");

    Migrator::up(&db, None)
        .await
        .expect("failed to run database migrations");

    // Share a single connection pool so handlers stay cheap and async-friendly.
    let state = AppState { db };

    let app = Router::new()
        .route("/tasks", post(create_task).get(list_tasks))
        .route("/tasks/{id}", get(show_task).delete(delete_task))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn create_task(
    State(state): State<AppState>,
    Json(payload): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<CreateTaskResponse>), (StatusCode, String)> {
    let CreateTaskRequest {
        task_type: raw_task_type,
        execution_time,
    } = payload;

    // Normalize upfront so the DB only sees task types we execute.
    let task_type = TaskType::parse(&raw_task_type).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid task type: {raw_task_type}"),
        )
    })?;

    let new_task = tasks::ActiveModel {
        id: Set(Uuid::new_v4()),
        task_type: Set(task_type.to_string()),
        execution_time: Set(execution_time),
        // Start every task in pending so the worker owns lifecycle transitions.
        state: Set(tasks::TaskState::Pending),
    };

    let inserted = new_task.insert(&state.db).await.map_err(map_db_err)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateTaskResponse { id: inserted.id }),
    ))
}

async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<Vec<Task>>, (StatusCode, String)> {
    let ListTasksQuery {
        state: state_filter,
        task_type,
    } = query;

    // Compose filters lazily so we stay index-friendly for mixed predicates.
    let mut finder = tasks::Entity::find();

    if let Some(state_filter) = state_filter {
        finder = finder.filter(tasks::Column::State.eq(state_filter));
    }

    if let Some(task_type_filter) = task_type {
        // Validate the filter before touching the DB to fail fast for typos.
        let parsed = TaskType::parse(&task_type_filter).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("invalid task type: {task_type_filter}"),
            )
        })?;

        finder = finder.filter(tasks::Column::TaskType.eq(parsed.to_string()));
    }

    let records = finder.all(&state.db).await.map_err(map_db_err)?;
    let tasks = records.into_iter().map(Task::from).collect();

    Ok(Json(tasks))
}

async fn show_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Task>, (StatusCode, String)> {
    let record = tasks::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(map_db_err)?;

    match record {
        Some(task) => Ok(Json(task.into())),
        None => Err((StatusCode::NOT_FOUND, "task not found".into())),
    }
}

async fn delete_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let result = tasks::Entity::delete_by_id(id)
        .exec(&state.db)
        .await
        .map_err(map_db_err)?;

    if result.rows_affected == 0 {
        Err((StatusCode::NOT_FOUND, "task not found".into()))
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

fn map_db_err(err: DbErr) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("database error: {err}"),
    )
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

impl fmt::Display for TaskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            TaskType::Foo => "foo",
            TaskType::Bar => "bar",
            TaskType::Baz => "baz",
        };

        f.write_str(value)
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
