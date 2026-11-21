PLATFORM ?= linux/amd64
IMAGE ?= shopping-sessions-rust:local
GENERATOR_IMAGE ?= shopping-sessions-generator:local
CHECKER_IMAGE ?= shopping-sessions-checker:local
NCPU ?= 6
MEM_GB ?= 8
DATA ?= data/dev
RUN ?= runs/dev
export IMAGE NCPU MEM_GB

.PHONY: build generator checker dev sample run fixtures check-dev
build:
	docker build --platform $(PLATFORM) -t $(IMAGE) .

generator:
	docker build --platform $(PLATFORM) -t $(GENERATOR_IMAGE) tools/generator

checker:
	docker build --platform $(PLATFORM) -f scripts/Dockerfile.checker -t $(CHECKER_IMAGE) scripts

dev: data/dev/manifest.json
data/dev/manifest.json: | generator
	mkdir -p data
	docker run --rm --platform $(PLATFORM) --network none --user "$$(id -u):$$(id -g)" \
	  -v "$(CURDIR)/data:/datasets" $(GENERATOR_IMAGE) --seed 1 --events 1000000 --clicks 250000 --output /datasets/dev

sample: data/sample/manifest.json
data/sample/manifest.json: | generator
	mkdir -p data
	docker run --rm --platform $(PLATFORM) --network none --user "$$(id -u):$$(id -g)" \
	  -v "$(CURDIR)/data:/datasets" $(GENERATOR_IMAGE) --seed 42 --events 10000 --clicks 2500 --output /datasets/sample

run: build
	mkdir -p runs
	sh scripts/run.sh "$(DATA)" "$(RUN)"

fixtures: build checker
	sh scripts/fixtures.sh "$(IMAGE)" "$(CHECKER_IMAGE)"

check-dev: checker
	docker run --rm --platform $(PLATFORM) --network none --read-only -e PYTHONDONTWRITEBYTECODE=1 \
	  -v "$(abspath $(RUN))/output:/actual:ro" -v "$(CURDIR)/checks/dev-answer:/expected:ro" \
	  $(CHECKER_IMAGE) compare.py /actual /expected
