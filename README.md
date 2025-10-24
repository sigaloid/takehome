## Summary
- Add an Axum-based API listener that persists scheduled tasks with Sea-ORM, normalizes incoming task types, and exposes create/list/show/delete endpoints backed by a PostgreSQL migration-driven schema.
- Build a Tokio-powered worker that claims pending work with `FOR UPDATE SKIP LOCKED`, drives the pending → running → completed/failed state machine, and executes the Foo/Bar/Baz behaviors (sleep, external GET via Reqwest, random number).
- Provide a `task-tester` smoke harness that issues tasks against the listener, waits for the worker to run them, and asserts list filters, scheduling guarantees, and delete semantics.

## Implementation Notes
- Sea-ORM and its migration crate define the `tasks` table plus a Postgres enum for `pending|running|completed|failed`, keeping schema management explicit and versioned.
- Axum + Tokio were chosen for the listener for their ergonomics and async performance; a shared connection pool keeps handlers lightweight while input validation caps accepted task types to the ones the worker understands.
- The worker uses Tokio for async polling, `tracing` for structured logs, and a transactional claim step with SKIP LOCKED so multiple instances can run without double-processing.
- Foo tasks sleep three seconds before printing their ID, Bar tasks make an HTTP GET to whattimeisitrightnow.com using Reqwest with rustls, and Baz tasks print a random number in the 0–343 range.


## Testing
- Not run in this environment (requires PostgreSQL); expected flow is `docker compose up postgres`, `DATABASE_URL=… cargo run -p api-listener`, `DATABASE_URL=… cargo run -p task-worker`, then `TASK_TESTER_BASE_URL=http://127.0.0.1:3000 cargo run -p task-tester`.

## Follow-ups
- Add retries or dead-letter handling for failed Bar calls instead of leaving tasks in `failed`.
- Introduce pagination and sorting controls on `/tasks` for large queues.
- Consider authentication, input rate limiting, and metrics to harden the API for production use.
