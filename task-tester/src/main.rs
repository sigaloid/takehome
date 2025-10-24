use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let base_url = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("TASK_TESTER_BASE_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:3000".to_owned());

    let tester = TaskTester::new(base_url)?;
    tester.run().await
}

struct TaskTester {
    client: Client,
    base_url: String,
}

impl TaskTester {
    fn new(base_url: String) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self { client, base_url })
    }

    async fn run(&self) -> Result<()> {
        println!("🔎 Connecting to API listener at {}", self.base_url);

        // Quick smoke check so we fail fast when the listener is unavailable.
        let existing = self
            .list_tasks(None, None)
            .await
            .context("unable to list existing tasks; is the API listener running and reachable?")?;
        println!("Found {} existing task(s) before starting.", existing.len());

        let start = Utc::now();
        let scenarios = vec![
            ScenarioPlan {
                task_type: "foo".to_owned(),
                execution_time: start + ChronoDuration::seconds(5),
            },
            ScenarioPlan {
                task_type: "bar".to_owned(),
                execution_time: start + ChronoDuration::seconds(6),
            },
            ScenarioPlan {
                task_type: "baz".to_owned(),
                execution_time: start + ChronoDuration::seconds(7),
            },
        ];

        println!("📝 Creating one task for each supported type (Foo, Bar, Baz)...");
        let mut scheduled = Vec::new();
        for plan in scenarios {
            let id = self
                .create_task(&plan.task_type, plan.execution_time)
                .await
                .with_context(|| format!("failed to create {} task", plan.task_type))?;

            println!(
                "  • Created {} task {id} scheduled for {}",
                plan.task_type, plan.execution_time
            );

            let task = self
                .get_task(id)
                .await
                .with_context(|| format!("failed to fetch task {id} after creation"))?;

            let delta_ms = (task.execution_time - plan.execution_time).num_milliseconds();
            if delta_ms.abs() > 1 {
                bail!(
                    "task {} stored execution time {} which differs from requested {}",
                    id,
                    task.execution_time,
                    plan.execution_time
                );
            }

            if !task.task_type.eq_ignore_ascii_case(&plan.task_type) {
                bail!(
                    "task {} stored type {} which differs from requested {}",
                    id,
                    task.task_type,
                    plan.task_type
                );
            }

            if task.state != TaskState::Pending {
                bail!(
                    "newly created task {id} unexpectedly in {} state",
                    task.state
                );
            }

            scheduled.push(ManagedTask {
                id,
                task_type: plan.task_type,
                execution_time: plan.execution_time,
            });
        }

        println!("📋 Verifying listing endpoints reflect newly created tasks...");
        let pending = self
            .list_tasks(Some(TaskState::Pending), None)
            .await
            .context("failed to list pending tasks")?;
        for managed in &scheduled {
            ensure_contains(&pending, managed.id).with_context(|| {
                format!(
                    "pending task list does not include recently created task {}",
                    managed.id
                )
            })?;
        }

        for managed in &scheduled {
            let matching_type = self
                .list_tasks(None, Some(&managed.task_type))
                .await
                .with_context(|| {
                    format!(
                        "failed to list tasks filtered by type {}",
                        managed.task_type
                    )
                })?;

            ensure_contains(&matching_type, managed.id).with_context(|| {
                format!(
                    "type-filtered list for {} is missing task {}",
                    managed.task_type, managed.id
                )
            })?;
            ensure_all_type(&matching_type, &managed.task_type)?;

            let combined = self
                .list_tasks(Some(TaskState::Pending), Some(&managed.task_type))
                .await
                .with_context(|| {
                    format!(
                        "failed to list tasks filtered by state and type ({}, pending)",
                        managed.task_type
                    )
                })?;
            ensure_contains(&combined, managed.id).with_context(|| {
                format!(
                    "combined filter (pending, {}) missing task {}",
                    managed.task_type, managed.id
                )
            })?;
            ensure_all_state(&combined, TaskState::Pending)?;
            ensure_all_type(&combined, &managed.task_type)?;
        }

        println!(
            "⏳ Waiting to ensure tasks stay pending before their scheduled execution time..."
        );
        for managed in &scheduled {
            self.assert_pending_until(managed, ChronoDuration::milliseconds(200))
                .await
                .with_context(|| {
                    format!("task {} transitioned before its execution time", managed.id)
                })?;
        }

        println!("⚙️  Polling until tasks complete...");
        let mut completed = Vec::new();
        for managed in &scheduled {
            let task = self
                .wait_for_completion(managed, Duration::from_secs(45))
                .await
                .with_context(|| format!("task {} did not complete in time", managed.id))?;
            println!("  • Task {} finished in {}", managed.id, task.state);
            if task.state != TaskState::Completed {
                bail!(
                    "task {} ended in unexpected {} state",
                    managed.id,
                    task.state
                );
            }
            completed.push(task);
        }

        println!("🧹 Confirming completed tasks disappear from the pending queue...");
        let pending_after = self
            .list_tasks(Some(TaskState::Pending), None)
            .await
            .context("failed to list pending tasks after execution")?;
        for managed in &scheduled {
            if pending_after.iter().any(|task| task.id == managed.id) {
                bail!(
                    "task {} still appears in pending list after completion",
                    managed.id
                );
            }
        }

        println!("✅ Verifying completed tasks show up in completed listings...");
        let completed_list = self
            .list_tasks(Some(TaskState::Completed), None)
            .await
            .context("failed to list completed tasks")?;
        for task in &completed {
            ensure_contains(&completed_list, task.id).with_context(|| {
                format!(
                    "completed tasks list missing task {} (type {})",
                    task.id, task.task_type
                )
            })?;
        }

        println!("🗑️  Checking delete flow...");
        let delete_id = self
            .create_task("foo", Utc::now() + ChronoDuration::seconds(30))
            .await
            .context("failed to create task for delete test")?;
        self.delete_task(delete_id)
            .await
            .with_context(|| format!("failed to delete task {}", delete_id))?;
        match self.try_get_task(delete_id).await {
            Ok(Some(_)) => bail!("deleted task {} is still retrievable", delete_id),
            Ok(None) => {
                // Expected outcome.
            }
            Err(err) => return Err(err),
        }

        println!("🎉 All API contract checks passed!");
        Ok(())
    }

    async fn create_task(&self, task_type: &str, execution_time: DateTime<Utc>) -> Result<Uuid> {
        let url = self.url("/tasks");
        let payload = CreateTaskRequest {
            task_type: task_type.to_owned(),
            execution_time,
        };
        let response = self
            .client
            .post(url)
            .json(&payload)
            .send()
            .await
            .context("failed to send create task request")?;
        let response = response
            .error_for_status()
            .context("task creation returned error status")?;
        let body: CreateTaskResponse = response
            .json()
            .await
            .context("failed to decode create task response")?;
        Ok(body.id)
    }

    async fn try_get_task(&self, id: Uuid) -> Result<Option<Task>> {
        let url = self.url(&format!("/tasks/{id}"));
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("failed to send show task request")?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let response = response
            .error_for_status()
            .context("show task returned error status")?;
        let body = response
            .json()
            .await
            .context("failed to decode show task response")?;
        Ok(Some(body))
    }

    async fn get_task(&self, id: Uuid) -> Result<Task> {
        self.try_get_task(id)
            .await
            .and_then(|opt| opt.ok_or_else(|| anyhow!("task {} not found", id)))
    }

    async fn list_tasks(
        &self,
        state: Option<TaskState>,
        task_type: Option<&str>,
    ) -> Result<Vec<Task>> {
        let mut request = self.client.get(self.url("/tasks"));

        if let Some(state) = state {
            request = request.query(&[("state", state.as_query_value())]);
        }

        if let Some(task_type) = task_type {
            request = request.query(&[("task_type", task_type)]);
        }

        let response = request
            .send()
            .await
            .context("failed to send list tasks request")?;
        let response = response
            .error_for_status()
            .context("list tasks returned error status")?;
        let tasks: Vec<Task> = response
            .json()
            .await
            .context("failed to decode list tasks response")?;
        Ok(tasks)
    }

    async fn delete_task(&self, id: Uuid) -> Result<()> {
        let url = self.url(&format!("/tasks/{id}"));
        let response = self
            .client
            .delete(url)
            .send()
            .await
            .context("failed to send delete task request")?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::NOT_FOUND => bail!("task {} not found for deletion", id),
            status => bail!("delete task returned unexpected status {status}"),
        }
    }

    async fn assert_pending_until(
        &self,
        task: &ManagedTask,
        tolerance: ChronoDuration,
    ) -> Result<()> {
        let check_time = task.execution_time - tolerance;
        let now = Utc::now();
        if check_time > now {
            let sleep_duration = to_std_duration(check_time - now)?;
            sleep(sleep_duration).await;
        }

        let snapshot = self.get_task(task.id).await?;
        if snapshot.state != TaskState::Pending {
            bail!(
                "task {} transitioned to {} before scheduled execution time {}",
                task.id,
                snapshot.state,
                task.execution_time
            );
        }
        Ok(())
    }

    async fn wait_for_completion(&self, task: &ManagedTask, timeout: Duration) -> Result<Task> {
        let deadline = Instant::now() + timeout;
        let mut first_non_pending: Option<DateTime<Utc>> = None;

        loop {
            let snapshot = self.get_task(task.id).await?;
            if snapshot.state != TaskState::Pending && first_non_pending.is_none() {
                first_non_pending = Some(Utc::now());
            }

            if let Some(change_time) = first_non_pending {
                let earliest_allowed = task.execution_time - ChronoDuration::milliseconds(200);
                if change_time < earliest_allowed {
                    bail!(
                        "task {} began processing too early ({change_time} < {earliest_allowed})",
                        task.id
                    );
                }
            }

            match snapshot.state {
                TaskState::Completed | TaskState::Failed => return Ok(snapshot),
                TaskState::Pending | TaskState::Running => {
                    if Instant::now() > deadline {
                        bail!(
                            "timed out waiting for task {} to finish; last state {:?}",
                            task.id,
                            snapshot.state
                        );
                    }
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }
}

#[derive(Debug, Clone)]
struct ScenarioPlan {
    task_type: String,
    execution_time: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ManagedTask {
    id: Uuid,
    task_type: String,
    execution_time: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct CreateTaskRequest {
    task_type: String,
    execution_time: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateTaskResponse {
    id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
struct Task {
    id: Uuid,
    task_type: String,
    execution_time: DateTime<Utc>,
    state: TaskState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TaskState {
    Pending,
    Running,
    Completed,
    Failed,
}

impl TaskState {
    fn as_query_value(&self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Running => "running",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
        }
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_query_value())
    }
}

fn ensure_contains(tasks: &[Task], id: Uuid) -> Result<()> {
    if tasks.iter().any(|task| task.id == id) {
        Ok(())
    } else {
        bail!("task {id} missing from response")
    }
}

fn ensure_all_state(tasks: &[Task], state: TaskState) -> Result<()> {
    if tasks.iter().all(|task| task.state == state) {
        Ok(())
    } else {
        bail!("response contains task with unexpected state")
    }
}

fn ensure_all_type(tasks: &[Task], task_type: &str) -> Result<()> {
    if tasks
        .iter()
        .all(|task| task.task_type.eq_ignore_ascii_case(task_type))
    {
        Ok(())
    } else {
        bail!("response contains task with unexpected type")
    }
}

fn to_std_duration(duration: ChronoDuration) -> Result<Duration> {
    duration
        .to_std()
        .map_err(|_| anyhow!("negative duration when converting to std::time::Duration"))
}
