set dotenv-load := false

db-url := env_var_or_default("EH_DATABASE_URL", "postgres://postgres:eh_test@localhost:5432/engagement_hub_db")

# Start Postgres 16 (background) and wait for healthy
db-up:
    docker compose up -d postgres
    @docker compose exec -T postgres bash -c 'until pg_isready -U postgres -d engagement_hub_db; do sleep 1; done'

# Stop + remove the Postgres volume (fresh schema next run)
db-reset:
    docker compose down -v
    just db-up
    just migrate

# Apply migrations via sqlx-cli (cargo install sqlx-cli --no-default-features --features postgres,rustls)
migrate:
    DATABASE_URL={{db-url}} sqlx migrate run --source migrations

# Convenience: bring DB up then run the workspace tests
test:
    just db-up
    EH_DATABASE_URL={{db-url}} cargo test --workspace

# Run the binary in dev mode (requires db-up first)
run-dev:
    EH_ENV=dev \
    EH_REGISTRY_ADAPTER=stub \
    EH_DATABASE_URL={{db-url}} \
    cargo run -p engagement-hub

# Regenerate Connect-Go stubs from both proto packages (requires buf 1.69+)
buf-gen:
    buf generate
