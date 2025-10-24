use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::extension::postgres::Type;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m_20251023_000001_create_tasks_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_type(
                Type::create()
                    .as_enum(TaskStateEnum::Table)
                    .values(TaskStateEnum::values())
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Tasks::Table)
                    .col(ColumnDef::new(Tasks::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Tasks::TaskType).string().not_null())
                    .col(
                        ColumnDef::new(Tasks::ExecutionTime)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Tasks::State)
                            .enumeration(TaskStateEnum::Table, TaskStateEnum::values())
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Tasks::Table).to_owned())
            .await?;

        manager
            .drop_type(
                Type::drop()
                    .if_exists()
                    .name(TaskStateEnum::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(Clone, Copy, Iden)]
enum Tasks {
    Table,
    Id,
    TaskType,
    ExecutionTime,
    State,
}

#[derive(Clone, Copy, Iden)]
enum TaskStateEnum {
    #[iden = "task_state"]
    Table,
    #[iden = "pending"]
    Pending,
    #[iden = "running"]
    Running,
    #[iden = "completed"]
    Completed,
    #[iden = "failed"]
    Failed,
}

impl TaskStateEnum {
    fn values() -> [Self; 4] {
        [
            TaskStateEnum::Pending,
            TaskStateEnum::Running,
            TaskStateEnum::Completed,
            TaskStateEnum::Failed,
        ]
    }
}
