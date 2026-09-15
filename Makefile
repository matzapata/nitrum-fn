.PHONY: lint format fmt-check check audit test adapters e2e e2e-cloud ci \
	stack stack-down images api publish-worker host oracle-contracts

# Override the registry/tag per target, e.g.:
#   make api IMAGE_PREFIX=ghcr.io/you/nitrum-fn TAG=sha-1234
#   make api API_IMAGE=docker.io/you/api:custom
IMAGE_PREFIX ?= ghcr.io/matzapata/nitrum-fn
TAG ?= dev
API_IMAGE ?= $(IMAGE_PREFIX)/api:$(TAG)
WORKER_IMAGE ?= $(IMAGE_PREFIX)/publish-worker:$(TAG)
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

e2e:
	bash tests/e2e/local.sh

e2e-cloud:
	./tests/e2e/cloud.sh

oracle-contracts:
	cd examples/oracle/contracts && forge test

ci: check audit test adapters e2e

stack:
	docker compose up -d --remove-orphans floci
	docker compose run --rm aws-init

stack-down:
	docker compose down --remove-orphans

images: api publish-worker host

api:
	docker buildx build --platform linux/amd64 -f Dockerfile.api -t $(API_IMAGE) .

publish-worker:
	docker buildx build --platform linux/amd64 -f Dockerfile.publish-worker -t $(WORKER_IMAGE) .

host:
	docker buildx build --platform linux/amd64 -f Dockerfile -t $(HOST_IMAGE) .
