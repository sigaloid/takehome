# API listener

This implements the task-based API listener. It connects to the database via the DATABASE_URL environment variable, using Sea-ORM as the ORM framework. This lets us construct cleaner abstractions for queries.

## How to run the API listener

Once PostgreSQL is running, you can run the API listener by executing the following command:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/db cargo run
```

## Few design decisions

Axum: battle-tested, high performance, ergonomic
Sea-ORM: Powerful way of expressing queries and models, plus nice migrations
Tokio: accepted async library
