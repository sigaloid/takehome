# Task worker

This implements the background task worker. It connects to the database via the DATABASE_URL environment variable, using Sea-ORM to pluck the next runnable task with SKIP LOCKED semantics before pushing it through the job executor.

## How to run the task worker

Once PostgreSQL is running, you can run the task worker by executing the following command:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/db cargo run
```

## Few design decisions

Tokio: async runtime lets the worker idle efficiently between polls
Sea-ORM: transaction helpers make it easy to safely tag tasks as running
Tracing: structured logs keep observability consistent with other services
Reqwest: lightweight HTTP client for outbound Bar tasks without pulling in OpenSSL

## Task grabbing details

Worker instances begin a transaction, select the earliest due task with `FOR UPDATE SKIP LOCKED`, and immediately flip its state to `running` before committing. That combination of row-level locking and state transition ensures other workers either see the task as already running or never acquire the lock, so a job is claimed exactly once and never before its scheduled execution time.
