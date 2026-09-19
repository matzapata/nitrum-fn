.PHONY: lint format fmt-check check audit test adapters ci \
	stack stack-down images api host

# Override the registry/tag per target, e.g.:
#   make api IMAGE_PREFIX=ghcr.io/you/nitrum-fn TAG=sha-1234
#   make api API_IMAGE=docker.io/you/api:custom
IMAGE_PREFIX ?= ghcr.io/matzapata/nitrum-fn
TAG ?= dev
API_IMAGE ?= $(IMAGE_PREFIX)/api:$(TAG)
HOST_IMAGE ?= $(IMAGE_PREFIX)/host:$(TAG)

lint:
	cargo clippy --workspace --all-targets -- -D warnings

format:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check: fmt-check lint

audit:
	cargo audit

test:
	cargo test --workspace --lib --bins

adapters:
	docker compose up -d --remove-orphans floci
	NITRUM_FN_ARTIFACTS__ENDPOINT=http://127.0.0.1:4566 \
	NITRUM_FN_CATALOG__ENDPOINT=http://127.0.0.1:4566 \
	AWS_REGION=us-east-1 AWS_DEFAULT_REGION=us-east-1 \
	AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test \
		cargo test --tests -p catalog -p artifacts

ci: check audit test adapters
	bash tests/e2e/local.sh

stack:
	docker compose up -d --remove-orphans floci
	docker compose run --rm aws-init

stack-down:
	docker compose down --remove-orphans

images: api host

api:
	docker buildx build --platform linux/amd64 -f Dockerfile.api -t $(API_IMAGE) .

host:
	docker buildx build --platform linux/amd64 -f Dockerfile -t $(HOST_IMAGE) .
