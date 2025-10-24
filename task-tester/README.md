# Task tester

This is a quick, LLM-assisted smoke test that exercises the task API end to end by creating the Foo, Bar, and Baz scenarios and verifying they behave as expected. it is NOT intended to be a fully comprehensive behavior or stress test. Very quick to confirm basic assumptions.

## How to run the task tester

Point it at a running API listener by passing the base URL as the first argument (or via the `TASK_TESTER_BASE_URL` environment variable) and execute:

```bash
TASK_TESTER_BASE_URL=http://127.0.0.1:3000 cargo run
```

If you omit the environment variable and argument, it defaults to `http://127.0.0.1:3000`.
