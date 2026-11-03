hooks: ## Enable the pre-commit hook (fmt + clippy)
	git config core.hooksPath .githooks

.PHONY: hooks up down dev worker scheduler migrate migration sqlx-prepare fmt lint test coverage docker-build ci

up: ## Start infra containers (postgres, valkey, rabbitmq, rustfs, mailpit)
	docker compose up -d

down: ## Stop containers
	docker compose down

dev: ## Run the API locally (needs `make up`)
	cargo run -- serve

worker: ## Run the RabbitMQ worker locally
	cargo run -- worker

scheduler: ## Run the cron scheduler locally
	cargo run -- scheduler

migrate: ## Apply database migrations
	cargo run -- migrate

migration: ## Create a migration: make migration name=add_something
	@test -n "$(name)" || (echo "usage: make migration name=<name>" && exit 1)
	sqlx migrate add -r $(name) 2>/dev/null || sqlx migrate add $(name)

sqlx-prepare: ## Refresh offline query metadata in .sqlx/ (commit the result)
	cargo sqlx prepare -- --all-targets

fmt:
	cargo fmt --all

lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings

test: ## Needs `make up`
	cargo test

coverage: ## Needs cargo-llvm-cov
	cargo llvm-cov --html --output-dir coverage

docker-build:
	docker build -t rust-template-app:local .

ci: lint test
