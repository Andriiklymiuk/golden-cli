# golden CLI release helper — run from the repo root. Mirrors corgi's Makefile.
#
#   make incrementVersionPatch     # then: git commit -am "release" && git push
#
# `crates/golden-cli/Cargo.toml` is the single source of truth for the version; it
# drives the release tag pushed by .github/workflows/tag.yml. Each bump also syncs
# the Claude plugin's version to match (corgi-style), so CLI + plugin never drift.

CLI_MANIFEST    := crates/golden-cli/Cargo.toml
PLUGIN_MANIFEST := plugins/golden/.claude-plugin/plugin.json

# The sample collection is embedded into the binary (include_str!) AND kept on
# disk for the dogfood job + integration tests. The on-disk copy is derived from
# the embedded asset; they must stay byte-identical (CI gates this via checkSample).
SAMPLE_ASSET := crates/golden-cli/assets/sample-collection.json
SAMPLE_DISK  := collections/sample-collection.json

VERSION := $(shell grep -E -o '^version = "[^"]*"' $(CLI_MANIFEST) | head -1 | awk -F '"' '{print $$2}')

getVersion:
	@echo $(VERSION)

# Used by .github/workflows/tag.yml — exports the CLI VERSION into the job env.
getActionVersion:
	@if [ -n "$(GITHUB_ENV)" ]; then \
		echo "VERSION=$(VERSION)" >> "$(GITHUB_ENV)"; \
	else \
		echo "GITHUB_ENV not set"; \
	fi

incrementVersionPatch:
	$(eval PATCH=$(shell echo $(VERSION) | cut -d '.' -f 3))
	$(eval NEW_PATCH=$(shell echo $$(($(PATCH) + 1))))
	sed -i "" 's/^version = "\([0-9]*\.[0-9]*\.\)$(PATCH)"/version = "\1$(NEW_PATCH)"/' $(CLI_MANIFEST)
	@$(MAKE) -s syncPluginVersion

incrementVersionMinor:
	$(eval MINOR=$(shell echo $(VERSION) | cut -d '.' -f 2))
	$(eval PATCH=$(shell echo $(VERSION) | cut -d '.' -f 3))
	$(eval NEW_MINOR=$(shell echo $$(($(MINOR) + 1))))
	sed -i "" 's/^version = "\([0-9]*\.\)$(MINOR)\.$(PATCH)"/version = "\1$(NEW_MINOR).0"/' $(CLI_MANIFEST)
	@$(MAKE) -s syncPluginVersion

incrementVersionMajor:
	$(eval MAJOR=$(shell echo $(VERSION) | cut -d '.' -f 1))
	$(eval NEW_MAJOR=$(shell echo $$(($(MAJOR) + 1))))
	sed -i "" 's/^version = "$(MAJOR)\.[0-9]*\.[0-9]*"/version = "$(NEW_MAJOR).0.0"/' $(CLI_MANIFEST)
	@$(MAKE) -s syncPluginVersion

# Force the Claude plugin's version (plugins/golden/.claude-plugin/plugin.json) to
# match the current CLI VERSION. Called by every incrementVersion* target via a
# sub-make, so VERSION reflects the just-applied bump (not the pre-bump value).
# Only the top-level "version" key is touched.
syncPluginVersion:
	sed -i "" 's/"version": "[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*"/"version": "$(VERSION)"/' $(PLUGIN_MANIFEST)
	@echo "golden CLI + plugin -> $(VERSION)"

# Copy the embedded sample asset onto the on-disk copy so the two never drift.
# Run this whenever you edit crates/golden-cli/assets/sample-collection.json.
syncSampleCollection:
	@mkdir -p $(dir $(SAMPLE_DISK))
	@cp $(SAMPLE_ASSET) $(SAMPLE_DISK)
	@echo "synced $(SAMPLE_DISK) <- $(SAMPLE_ASSET)"

# CI gate: fail if the two sample copies differ (someone edited one, not both).
checkSampleCollection:
	@if cmp -s $(SAMPLE_ASSET) $(SAMPLE_DISK); then \
		echo "sample collection in sync ($(SAMPLE_ASSET) == $(SAMPLE_DISK))"; \
	else \
		echo "ERROR: $(SAMPLE_DISK) differs from $(SAMPLE_ASSET). Run 'make syncSampleCollection'." >&2; \
		exit 1; \
	fi

.PHONY: getVersion getActionVersion \
	incrementVersionPatch incrementVersionMinor incrementVersionMajor \
	syncPluginVersion syncSampleCollection checkSampleCollection
