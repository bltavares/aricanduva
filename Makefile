ifeq ($(OS),Windows_NT)
	SHELL := pwsh.exe
	.SHELLFLAGS := -NoProfile -Command
endif

TMP ?= $(or $(TMPDIR), /tmp)
export DATABASE_PATH := $(TMP)/aricanduva.test.db
export DATABASE_URL := sqlite:$(DATABASE_PATH)

# development preparations
prepare:
	cargo bin --install
	cargo sqlx database reset -y
	cargo sqlx prepare

# Run the linter to check for code issues
lint: | prepare
	cargo clippy -- -D clippy::pedantic -A clippy::items-after-statements
	
# Run in development with listenfd
run: export RUST_LOG ?= aricanduva=debug
run:
	cargo bin systemfd --no-pid -s http::[::]:3000 -- cargo bin watchexec cargo run

DOCKER_IMAGE := bltavares/aricanduva
# Build container
docker:
	docker build . -t $(DOCKER_IMAGE)

# Remove build files
clean:
	-rm -r target/
	-rm $(DATABASE_PATH)

.PHONY: lint clean prepare docker run
