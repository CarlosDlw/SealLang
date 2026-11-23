# Acceptance matrix

This page is the executable contract of the modules-and-projects work. Every
row names the test or command that proves it. The suite is run in one command:

```console
$ cargo fmt --all -- --check
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo build --workspace
$ cargo test --workspace -j 1 -- --test-threads=1
$ scripts/verify-commit.sh HEAD
```

The CI gates are the same commands. `ci.yml` runs formatting, clippy, build,
tests, and reproducibility on every push and pull request; `acceptance.yml`
runs the serial suite and reproducibility on `main`, nightly, and manual
dispatch; `install.yml` runs the installer acceptance test. See
[`ci.md`](ci.md).

Test paths are relative to the repository root. A test name without a path is
inside the file named in the same row.

| # | Criterion | Evidence |
|---|-----------|----------|
| A1 | `module a::b;` merges items across files, parents are implicit, the root needs no declaration | `crates/hir/src/tests/modules.rs::a_file_without_a_declaration_belongs_to_the_package_root`, `::one_module_merges_its_fragments`; `examples/multi-module` |
| A2 | Absolute `use`, groups, aliases, file scope, no glob | `crates/hir/src/tests/imports.rs::an_import_can_start_at_the_package_root`, `::a_group_alias_renames_only_its_own_binding`, `::imports_are_not_shared_between_fragments_of_one_module`; `crates/parser/src/tests/imports.rs::glob_imports_are_rejected` |
| A3 | `public`/`internal`/`private` on items, fields, and methods; invalid positions rejected | `crates/parser/src/tests/visibility.rs` (all cases); `crates/hir/src/tests/modules.rs::a_private_item_cannot_be_imported_from_another_module` |
| A4 | Fragment duplicates, ambiguity, unused import, module cycle | `crates/hir/src/tests/modules.rs::two_fragments_cannot_declare_the_same_name`; `crates/hir/src/tests/imports.rs::a_name_that_is_both_a_module_and_an_item_is_ambiguous`, `::an_import_that_resolution_never_uses_is_a_warning`, `::modules_may_import_each_other_in_a_cycle` |
| A5 | Public API is `[package.api] expose`; leaking a private type is an error | `crates/hir/src/tests/api.rs` (all cases); `crates/cli/tests/workspace_cli.rs::doc_and_api_include_docs_layout_and_cfg` |
| A6 | Symbols do not collide across packages or instances; internals stay hidden | `crates/codegen/tests/exports.rs`; `crates/build/tests/build.rs::cgu_splits_are_parallel_and_only_the_changed_unit_reruns`; `crates/cli/tests/project_cli.rs::per_edge_features_build_two_instances_of_one_package` |
| A7 | `seal new/init/build/run/check/metadata` on a multi-file project | `crates/cli/tests/project_cli.rs::a_new_binary_project_builds_checks_and_runs`, `::a_compiler_template_builds_the_multi_file_pipeline` |
| A8 | Root manifest, members, targets, entry; validation with spans | `crates/manifest/src/validate.rs` (all cases); `crates/cli/tests/project_cli.rs::metadata_is_parseable_and_lists_modules_and_targets` |
| A9 | `sealc build file.sl` unchanged; `--plan` equivalent | `crates/driver/tests/`; `crates/typeck/src/tests/examples.rs::all_examples_type_check`; `crates/codegen/tests/end_to_end.rs` |
| A10 | Per-edge features: two instances, no unification; conflict/cap | `crates/project/src/tests/features.rs::dependencies_with_different_features_get_distinct_instances`, `::equal_feature_sets_share_one_instance`, `::conflicting_features_are_reported`, `::the_feature_variant_cap_reports_the_limit`; `examples/features` |
| A11 | Typed `cfg` on items, modules, and imports; inactive code is not checked | `crates/hir/src/tests/cfg.rs` (all cases); `crates/project/src/tests/cfg.rs::an_inactive_target_is_not_compiled` |
| A12 | Profiles and precedence; opt/debug/cgu/lto in the artifact | `crates/project/src/tests/profiles.rs`; `crates/cli/src/tests/build.rs::a_target_profile_selects_optimization_values`, `::a_command_line_profile_overrides_the_target_profile` |
| A13 | Path deps, package cycle, deterministic lockfile, `--locked`/`--frozen` | `crates/project/tests/project.rs::a_dependency_cycle_is_reported`, `::the_lockfile_is_deterministic_and_relative`; `crates/cli/tests/registry_cli.rs::add_vendor_offline_build_audit_sbom_and_queries` |
| A14 | Per-package orphan rule and cross-package coherence | `crates/hir/src/tests/packages.rs` (all cases) |
| A15 | Content-addressed action graph; no-op full hit; cache verify/prune | `crates/build/tests/build.rs::a_repeated_build_is_a_full_cache_hit`, `::persistent_frontend_payloads_are_keyed_by_source_content`; `crates/cli/tests/project_cli.rs::quiet_suppresses_status_and_color_is_explicit` |
| A16 | Sandbox blocks undeclared reads and writes; env/HOME isolated | `crates/build/tests/build.rs::a_generator_cannot_read_an_undeclared_package_file`; `crates/sandbox` tests |
| A17 | Generator reruns only on input/tool change; undeclared output rejected | `crates/build/tests/build.rs::generators_run_only_when_their_inputs_or_tool_change`, `::a_generator_output_may_not_overwrite_a_source`; `examples/generator` |
| A18 | Native probes use pkg-config or explicit flags, cached, never silent | `crates/build/tests/build.rs::declared_native_libraries_are_recorded_and_linked`, `::pkg_config_native_dependencies_are_resolved_when_available`; `crates/manifest/src/validate.rs::a_native_library_name_cannot_smuggle_a_path` |
| A19 | `.sealmono` round-trip; deterministic CGUs; multi-object link | `crates/codegen/tests/mono.rs`; `crates/driver/tests/mono.rs`; `crates/build/tests/build.rs::job_count_does_not_change_the_artifact` |
| A20 | Two builds in different directories are byte-identical | `crates/cli/tests/project_cli.rs::verify_repro_builds_two_copies_and_finds_them_identical` |
| A21 | Pinned and verified toolchain; a wrong checksum fails | `crates/cli/tests/project_cli.rs::a_pinned_toolchain_is_verified_before_a_build`; `crates/toolchain/src/store.rs` tests |
| A22 | Freestanding `_start` exits 42 without runtime; panic traps | `crates/cli/tests/project_cli.rs::a_kernel_template_builds_freestanding_and_exits_42`; `crates/cli/tests/acceptance.rs::every_project_example_builds_and_runs`; `examples/freestanding` |
| A23 | Cross object/link with declared linker and sysroot | `crates/cli/tests/project_cli.rs::a_cross_binary_may_not_link_the_host_runtime`, `::a_target_spec_linker_is_used_by_the_build` |
| A24 | PubGrub resolution, verified download, offline vendor | `crates/registry/tests/resolution.rs` (all cases); `crates/cli/tests/registry_cli.rs::the_registry_example_vendors_and_runs_offline`; `examples/registry` |
| A25 | `add/remove/update`, lockfile v2, `--precise`, `--dry-run` | `crates/cli/tests/registry_cli.rs::add_vendor_offline_build_audit_sbom_and_queries`, `::precise_update_moves_a_locked_version`; `crates/project/src/tests/lockfile.rs` |
| A26 | Audit detects advisory/license/yanked/provenance; CycloneDX SBOM | `crates/cli/tests/registry_cli.rs::audit_reports_a_matching_advisory`, `::add_vendor_offline_build_audit_sbom_and_queries`; `crates/registry/src/audit.rs` tests |
| A27 | Publish, namespace, owner, immutability, yank | `crates/registry-server/src/publish.rs::a_publication_is_stored_with_valid_attestations`, `::an_existing_version_cannot_be_replaced`, `::a_namespace_must_be_owned_before_publishing`, `::yanking_records_a_log_entry_and_can_be_undone` |
| A28 | TUF resists rollback/freeze; signatures and provenance verified | `crates/registry/tests/tuf.rs` (all cases) |
| A29 | Transparency log with inclusion proof | `crates/registry-server/src/publish.rs` publication tests; `crates/registry/src/log.rs` tests |
| A30 | `-p`, `--workspace`, `--exclude`, `default-members` | `crates/cli/src/tests/build.rs::workspace_and_exclude_select_the_right_packages`, `::default_members_select_every_declared_member`; `crates/cli/tests/project_cli.rs::a_workspace_builds_every_member` |
| A31 | `graph/tree/why/affected` correct with stable JSON | `crates/cli/tests/workspace_cli.rs::the_versioned_monorepo_fixture_supports_workspace_queries`, `::graph_json_contains_nodes_and_edges`, `::affected_maps_a_source_change_to_its_package` |
| A32 | `doc`/`api`/`semver-check`; breaking change fails publish | `crates/cli/tests/workspace_cli.rs::doc_and_api_include_docs_layout_and_cfg`, `::semver_check_accepts_a_compatible_baseline_file`; `crates/api/src/semver.rs` tests |
| A33 | Test discovery, isolation, filters, sharding, retries, JUnit | `crates/cli/tests/test_runner.rs::tests_run_isolated_and_fail_with_the_process_status`, `::unit_tests_see_private_items_and_integration_tests_do_not_need_them`, `::stress_and_partitions_cover_every_test_once`, `::json_and_junit_reports_are_machine_readable` |
| A34 | `bench` and `watch` work; `doctor`/`config` explain the environment | `crates/cli/tests/test_runner.rs::benchmarks_run_and_report_wall_time`, `::watch_once_runs_the_requested_command_without_looping`, `::config_resolves_cli_environment_and_default_origins`, `::doctor_reports_the_extended_checks_and_rejects_unknown_lints` |
| A35 | Incremental edits are correct and measured; no incompatible reuse | `crates/query/tests/engine.rs`, `crates/query/tests/parallel.rs`; `crates/build/tests/build.rs::persistent_frontend_payloads_are_keyed_by_source_content`; [`architecture.md#measurements`](../architecture.md#measurements) |
| A36 | Remote cache/execution with integrity, auth, and fallback | `crates/cli/tests/remote_cli.rs::a_remote_hit_restores_a_clean_build_without_local_cache`, `::local_and_remote_cache_builds_produce_identical_artifacts`, `::a_remote_failure_falls_back_or_fails_when_required`, `::offline_mode_rejects_a_remote_endpoint` |
| A37 | Docs, migration, examples, and `language-gaps` updated | This page, [`../migration.md`](../migration.md), [`../../examples/`](../../examples/), and every project/language chapter linked below |
| A38 | `fmt`, `clippy -D warnings`, `build`, `test`, `verify-commit` green | The command block at the top of this page |
| A39 | No `TODO`/“planned” in shipped code; open doors justified | `rg -n "TODO\|unimplemented!\|todo!" crates`; [`architecture.md`](../architecture.md) extension points and the plan's open-door table |
| A40 | No secret or build artifact committed; `.gitignore` correct | `git ls-files` review; `.gitignore` excludes `.local/`, `target/`, caches, and generated `ztests` state |
| A41 | D29 targets measured (startup, no-op, graph, resolution) without regression | [`architecture.md#measurements`](../architecture.md#measurements) |
| A42 | Editions are per package; deprecation warns at every use and is lint-controlled | `crates/version/src/lib.rs::edition_parsing_is_strict_and_names_the_supported_line`, `::editions_order_by_year`; `crates/project/src/tests/edition.rs::changing_the_edition_changes_the_instance_hash`; `crates/parser/src/tests/deprecated.rs::deprecation_metadata_is_parsed_on_functions`, `::deprecation_is_rejected_on_modules_imports_impls_and_extern_blocks`, `::every_deprecatable_item_form_accepts_the_modifier`; `crates/hir/src/tests/deprecated.rs::every_resolved_hir_use_warns_at_its_use_span`, `::imports_and_reexports_warn_at_the_import_path`; `crates/typeck/src/tests/deprecated.rs::field_access_and_inherent_method_calls_warn_once`; `crates/driver/tests/diagnostics.rs::a_deprecation_warning_can_be_allowed_or_denied`; `crates/driver/tests/examples.rs::deprecation_example_warns_and_runs_at_both_optimization_levels`; `crates/cli/tests/acceptance.rs::the_deprecation_example_warns_on_use`; `crates/cli/tests/test_runner.rs::lint_maps_deprecated_to_the_individual_lint_flags`; `examples/deprecation` |

## Project examples

`crates/cli/tests/acceptance.rs` copies, builds, and runs every project
example. The registry consumer is covered by
`crates/cli/tests/registry_cli.rs::the_registry_example_vendors_and_runs_offline`,
which builds a local TUF registry, vendors it, deletes the registry, and runs
the program offline.

| Example | Result |
|---------|--------|
| [`multi-module`](../../examples/multi-module/) | exits 42 |
| [`workspace`](../../examples/workspace/) | exits 42 |
| [`features`](../../examples/features/) | exits 3 |
| [`generator`](../../examples/generator/) | exits 42 |
| [`freestanding`](../../examples/freestanding/) | exits 42 |
| [`registry`](../../examples/registry/) | exits 42 |
| [`deprecation`](../../examples/deprecation/) | warns on use, exits 42 |

## Definition of done

1. All 42 rows above are green.
2. All 14 phases of the plan are complete with deviations recorded where a
   different implementation was chosen.
3. `cargo fmt --all --check` has no diff.
4. `cargo clippy --workspace --all-targets -- -D warnings` has no warning.
5. `cargo build --workspace` succeeds from a clean state.
6. `cargo test --workspace -j 1 -- --test-threads=1` passes.
7. `scripts/verify-commit.sh HEAD` passes on the committed tree.
8. No new `TODO`, `unimplemented!`, `todo!`, or silent error branch exists in
   the delivered code; remaining panics are documented invariants.
9. No functional limitation is deferred without an explicit extension point.
10. Documentation and examples are updated in the same change; resolved entries
    are removed from the language-gap list.
11. No secret, build artifact, `target/`, `vendor/`, or cache is committed.
12. A multi-module project, a workspace, a features project, a generator, a
    freestanding target, and a registry consumer build and run in the suite.
13. `seal doctor` is green and `seal metadata` is consumable by an editor.
14. Reproducibility is verified in two distinct directories.

## Manual checks

```console
$ seal doctor --json
$ seal metadata --project examples/multi-module
$ seal verify-repro --project examples/multi-module
$ cp -R examples/multi-module /tmp/multi-module && seal run --project /tmp/multi-module
```
