# Agent kernel plan for Cortex Desktop

2026-11-19 · based on commit `39318b4` · line numbers are from that commit

This plan compares Cortex Desktop with the "AI Agent Kernel Tools" document (v2.0) and its `agent-kernel`
example, and sets out how to make the AI core more scalable and reusable.

## 1. Conclusion

Cortex already has most of the kernel's *features*: memory, skills, sub-agents, approvals, deferred tool
loading, scheduling, a code sandbox. What it lacks is the kernel's *shape*. Each feature is wired by hand
into one 730-line function, and the same concept is implemented several times.

So the kernel document's build order (sandbox first, then registry) is the wrong order for Cortex. A
registry or a WASM sandbox added now would cover one of two tool models and one of four agent loops. The
order here is:

1. Unify what exists: one tool model, one execution pipeline, one agent loop.
2. Separate it from Tauri, so every front end reuses it.
3. Then add the kernel pieces that are missing: traces, a real registry, the WASM sandbox, agent-written
   tools, evals and lifecycle.

Three things are missing outright and matter most: an **evaluation harness**, a **run trace the agent can
read**, and a way for the **model to create executable tools**.

## Progress (updated 2026-11-19)

| Phase | Status |
|---|---|
| 0. Quick fixes | Done, except the process limit (dropped, see phase 0). |
| 1. One tool pipeline | Done: `Host` seam, turn tests, handler table, one long-output store. |
| 2. One loop | Done: headless, terminal chat and `delegate` workers run through `engine::turn`; the system prompt is assembled from named, budgeted sections. |
| 3. Workspace split | Done (see phase 3): `cortex-kernel` holds the agent loop and its decisions; each step's I/O is the app's `TurnSteps`. |
| 4. Traces and registry | Done: SQLite traces, feedback and artifact registry; `usage_stats`, `failure_summary`; trace-based budgets; versions for memories, pages, notebooks, instructions. |
| 5. One retriever | Done for libraries, knowledge pages, tools and skills. Chat search keeps exact substring matching on purpose. |
| 6. WASM sandbox | Done: `create_tool` / `promote_tool`, a Boa JavaScript interpreter as a WASI component, capability checks, limits, hash pinning. |
| 7. Evals and lifecycle | Done: eval suites, compare, ablate, regressions, eval-gated promotion, `propose_change` with a review, a daily lifecycle job, opt-in nightly evals, one timing engine for both schedulers (cron tasks use the same library; yearly ones are computed by hand), and a Settings section for the artifact registry. |

All work is on `claude/agent-kernel-phase-0` (commit `2504bef`, then the fixes from seventeen independent audits). 756 app
tests (15 more need a model or the network and are ignored), 17 integration tests and 63 tests in the four crates pass (836).

## Checked against the kernel document

The kernel document was re-read on 2026-11-19 and every requirement checked against the code. Then an
independent audit, by a reviewer that had not written the code, checked every row again with file and
line evidence and flagged rows whose test did not prove the claim. Every gap it found was built. A
second through ninth independent audit re-checked every row the same way; what they found is fixed and
tested too: among it, written tools retired while in use, trials of a change that, run on the live
data, could leave it applied after a Stop, tools a turn had not offered still running, and a promotion
granting what was on disk rather than what the card showed, and a race that let a link swapped in
mid-call lead a tool's write outside its workspace. What tests cannot reach is listed in section 7. Each row names
where the requirement lives and the test that covers it.

`scripts/check-kernel-plan.py` keeps these tables, and section 7's table of the example's open points,
honest: it reads every test named in their last column,
fails if one does not exist, and runs them all. CI runs it on every push ("The kernel plan's evidence").

### The rules (document section 2)

| Rule | Where | Test |
|---|---|---|
| Growth on disk, pointers in context | What grows is retrieved, not listed: skills, facts and pages past their limits (`skills::prompt_for`, `memory::prompt_for`, `knowledge::recalled`), connector, OpenAPI, custom and written tools past 30 (`find_tools`); long results are references (`read_tool_output`); the prompt's sections keep to a budget (`cortex-kernel::prompt`). The fixed core (about 35 built-ins and the file, shell and browser connectors) is always offered, and tool schemas are not counted in the prompt budget | `many_skills_are_narrowed…`, `many_facts_become_a_short_index…`, `defers_only_past_the_limit…`, `over_budget_the_least_important…` |
| Measured against a set the agent does not control | Eval suites in `<config>/evals/`, out of every tool's reach. With suites, a tool's promotion, a skill (`save_skill`, also under an ask-first policy) and a skill, instruction or memory change (`propose_change`) are measured before the user is asked: in a throwaway copy of what shapes an answer (`evals::trial_folder`: the `chat`, `skills` and `evals` folders, without conversations, caches or local models, nor a link leading back up to the data folder; deleted however the turn ends), each suite runs, the change is made (a tool active with nothing granted, so a case runs it), each suite runs again; a suite that does worse is run once more before the change is held to have made it worse. The tool on trial reaches no network before the user approves (the eval cases' own reading tools, such as web search, run as in any eval), so what the tool fetches is judged later, by the monitor on its live calls. The live data is never touched, so a stopped turn, a declined change or two trials at once leave nothing, and trial runs are no baseline. Nightly and command-line eval runs also run on a copy, and only their scores come back as baselines, per model. The scores are on the approval card; the model hears only that it held or that it was not offered; a change that makes any suite worse is never offered | `the_eval_sets_traces_and_audit_log_are_out_of_reach`, `a_promotion_is_gated_on_the_user_s_evals`, `a_change_that_makes_the_evals_worse_is_never_offered`, `a_declined_skill_leaves_nothing_behind`, `a_trial_copy_is_partial_and_removed_when_dropped`, `a_link_back_up_to_the_data_folder_is_not_copied`, `nightly_evals_compare…`, `a_tool_is_tried_by_running_it_in_the_copy`, `one_unlucky_run_does_not_sink_a_change` |
| Every self-modification versioned, attributed, reversible | `versions.rs` for memories, pages, notebooks, instructions, agents, workflows, tools, scheduled tasks (the last 30 of each; all facts are one item); `versions::restore` brings any version back (`chat_restore_version`; History on the Knowledge page and in the agent, workflow and tool editors, each with Show changes through `chat_version_diff`), keeping the one it replaces (a written tool's script comes back, never more access than it has now, judged on its own calls from then on, and a version that does not verify comes back as a candidate); skill history; provenance on facts, skills and tools, not on version entries | `changing_memories_keeps_the_previous_version`, `a_memory_change_can_be_reverted`, `pages_instructions_and_notebooks_can_be_reverted`, `a_version_diffs_against_the_one_after_it`, `tampered_files_are_not_signed_and_restores_do_not_widen_access`, `a_task_edit_can_be_undone`, `a_version_restored_from_history_is_judged_on_its_own_calls` |

### The 14 tools (document section 4), operation by operation

| Tool | Operation or requirement | Where | Test |
|---|---|---|---|
| 1 Sandbox | `run(code, limits)` | `run_script`: a one-off JavaScript program in the WASM sandbox with nothing granted, within the time and memory asked for (capped; fuel stays at its default); `cortex-sandbox::run`; `run_code` for native code | `a_one_off_program_runs_within_its_limits_and_reaches_nothing`, `a_script_computes_and_returns_json` |
| | `load(binary, grants)`, `call(tool, args)` | `script_tools::promote`, `script_tools::call` | `a_tool_is_written_tested_promoted_and_pinned` |
| | No ambient files, network or environment | The WASI context is built empty, inheriting nothing (`WasiCtxBuilder::new().build()`, by construction); scripts get no ambient API, only capability-checked host functions: `fetch` (listed hosts, no proxy, private and reserved ranges refused, IPv4 inside IPv6 included), `read` and `write` (granted folders of the chat's workspace, each file opened by walking down from the workspace with `openat` and `O_NOFOLLOW`, so no link is followed even one swapped in mid-call, and a hard-linked file is refused (on Linux and macOS; on Windows links are checked path by path and hard links are not detected); 1 MiB), `tool` (listed tools) | `nothing_is_reachable_without_a_capability`, `wasi_gives_nothing`, `reads_stay_inside_the_granted_folders_of_the_workspace`, `a_read_cannot_follow_a_link_out_of_the_workspace`, `writes_stay_inside_the_granted_folders_of_the_workspace`, `reads_and_writes_are_capped`, `a_read_cannot_follow_a_link_into_an_ungranted_folder`, `a_link_swapped_in_mid_write_is_never_followed`, `a_hard_link_into_an_ungranted_folder_is_refused` |
| | CPU, memory, time limits; fresh instance per call | Fuel, `ResourceLimiter`, yielding epoch plus timeout (which also bounds a host call that never answers); a `Store` per call | `fuel_bounds_the_cpu`, `a_memory_hog_is_stopped`, `an_endless_loop_is_stopped`, `a_busy_tool_does_not_starve_the_runtime`, `a_host_call_that_never_returns_is_stopped`, `nothing_carries_over_between_calls` |
| | Compose tools in one script | `tool(name, args)` host function, capability `tools`, three levels deep, authorized by the caller's approved `tools` capability (a nested call does not pass the chat's hooks, saved policies or offered set, and is logged in the activity log as called by a tool); a tool's effect is the union of its callees', so one that calls a networked or writing tool is not offered as read-only and its call passes the gate as one that changes something | `a_script_calls_only_the_tools_it_may`, `a_tool_calls_another_approved_tool`, `tools_call_tools_three_levels_deep_at_most`, `a_tool_that_calls_a_networked_tool_is_not_read_only`, `a_tool_that_writes_is_granted_its_folder_and_changes_something`, `a_written_tool_s_runs_are_in_its_turn_s_trace` |
| | Compiled once, not per load | The compiled interpreter is kept in `<config>/cache/wasm`, keyed by the component's and the engine's hashes; a damaged copy is compiled again | `the_compiled_interpreter_is_kept_and_reused` |
| | A second tier for native code | `run_code`: the Podman workbench or bubblewrap. Where neither isolates it (macOS, Windows, no user namespaces), every run asks, in Auto too, and never runs unwatched | `code_runs_unsandboxed_only_with_the_user`, `unsandboxed_code_asks_even_in_auto` |
| 2 Storage | `read`, `write`, `edit`, `list`, `search` | The filesystem connector; the sandbox workspace; libraries | `reading_a_file_returns_its_contents`, `editing_a_file_replaces_matched_text_on_disk`, `content_search_skips_symlinks_that_escape_the_root`, `runs_code_in_the_workspace_and_lists_its_files` |
| | Separate areas for tools, skills, memory, evals, scratch | Separate folders: `chat/script-tools/`, `skills/`, `chat/memories.json`, `evals/`, `chat/sandbox/<chat>`. The test checks the one separation that is a guardrail: evals outside what tools write | `suites_live_outside_the_folder_tools_write` |
| | Per-user and shared scopes | Single-user app (section 8): scopes are everywhere, project and agent | `many_facts_become_a_short_index_and_scopes_apply` |
| 3 Registry | `register` | `create_tool` (candidate), `promote_tool` (active), which grants only what the card showed; where nobody can approve (Plan mode, scheduled runs, evals) `create_tool` cannot rewrite an approved tool | `a_tool_written_in_one_chat_is_approved_and_used_in_another`, `an_approved_tool_is_not_rewritten_where_nobody_can_approve`, `a_promotion_grants_only_what_the_card_showed` |
| | `search_tools`, `load_tool` | `find_tools`: BM25 over the deferred tools (connector, OpenAPI, custom and written), fused with a ranking by meaning when an embedding model is set up (each tool embedded once); loads what it finds | `tools_are_ranked_by_meaning_from_their_embeddings`, `meaning_and_words_are_fused`, `defers_only_past_the_limit…` |
| | `retire(name)` | `retire_tool` (under the approval mode), lifecycle, `script_tools::retire` | `a_tool_s_access_and_retirement_go_through_the_user`, `unused_assistant_tools_retire…` |
| | Namespaces, version history, status, content hash | `script__<name>`; `versions.rs`; candidate, active, deprecated on the tool, and archived in the artifact registry; the approval pins the full SHA-256 of the script, description and parameters; an HMAC signature over what was approved, keyed in the OS keychain, so a file changed outside the app is neither offered nor run, nor is an earlier state the app signed put back over it (each tool's latest signature is recorded in the keychain, outside the data folder) (the signature covers every field the monitor and History go by, its rollback and restore dates among them, and promoting such a file again takes those fields from the last state the app signed, kept apart from the tool's own file, rather than signing what was written; with no signed state left it is written again and tested afresh, not promoted (a tool the app never signed, written before the key was loaded, is promoted as written; one the app signed is not, whatever records were removed); a History restore takes the set-aside list from the last signed state too; demoting, revoking or retiring it leaves it unsigned), and no later change by the app signs it as approved | `a_tool_is_written_tested_promoted_and_pinned`, `only_the_assistant_s_unused…`, `a_tool_changed_outside_the_app_is_refused`, `the_signature_is_standard_hmac_sha256`, `the_signing_key_is_made_once_and_kept_in_the_keychain`, `a_forged_version_is_never_rolled_back_to`, `tampered_files_are_not_signed_and_restores_do_not_widen_access`, `every_field_the_monitor_or_history_goes_by_is_signed`, `a_promotion_does_not_sign_a_rollback_date_written_outside_the_app`, `a_promotion_does_not_sign_checks_emptied_outside_the_app`, `a_touched_restored_file_promoted_again_keeps_what_the_user_set_aside`, `checks_emptied_outside_the_app_come_back_without_history`, `retiring_a_changed_file_does_not_sign_its_rollback_date`, `revoking_a_changed_file_does_not_sign_its_rollback_date`, `a_changed_file_with_nothing_signed_left_is_written_again_not_promoted`, `a_tool_written_before_the_key_was_loaded_can_be_promoted`, `a_trial_promotes_a_tool_written_before_the_key`, `a_forged_history_date_is_not_signed`, `a_touched_file_then_a_restore_keeps_what_was_set_aside`, `an_earlier_signed_state_put_back_does_not_verify`, `a_stripped_tool_with_its_records_removed_is_not_promoted_as_never_signed`, `the_latest_signatures_are_kept_in_the_keychain`, `the_latest_signatures_are_kept_in_the_keychain` |
| | Search before create; update rather than duplicate | `create_tool` refuses a near-duplicate unless told `new_anyway` | `a_near_duplicate_is_refused_unless_asked_for` |
| 4 Evals | `run_eval`, `compare`, `ablate` | `evals::run_suite`, `compare`, `ablate`, leaving out tools, skills, facts, pages or the instructions (`tools_off`, `skills_off`, `facts_off`, `pages_off`, `instructions_off` on the turn); `cortex-desktop eval --without tool:|skill:|fact:|page:|instructions` | `an_eval_suite_runs_through_the_loop_and_is_graded`, `comparisons_name_the_cases_that_moved`, `ablating_a_skill_shows_whether_it_helps`, `ablating_a_fact_and_the_instructions_shows_what_they_add` |
| | Human-owned sets; a part hidden from the agent | Hidden from the agent's own reach: the filesystem connector, `run_code`, written tools and eval cases (which run on a copy of the data) cannot read or change `evals/`, and a shell command that names the app's data folder asks in every mode. A shell command can still reach it without naming it (a glob, for one), under the user's approval mode: a guard, not a wall, since the host shell is the user's own. The user writes, edits and deletes cases in Settings (Eval cases), which also turns a rated-down answer's case into one that runs | `the_eval_sets_traces_and_audit_log_are_out_of_reach`, `a_command_naming_the_app_s_data_always_asks`, `a_rated_down_answer_becomes_a_waiting_regression_case`, `cortex_own_directories_stay_out_of_reach_inside_an_allowed_folder` |
| | Regression suites per tool | Each script tool's checks: its good test run, re-run on every rewrite, at promotion and daily, on a fixed clock and seeded randomness (so a tool reading the date or drawing random numbers holds). Checks run with nothing granted, so they hold what a tool computes; what it fetches is judged by the monitor on its live calls | `regression_checks_hold_a_tool_to_what_it_did`, `the_test_run_has_no_capabilities_whatever_was_asked`, `a_check_on_a_tool_using_the_clock_holds_tomorrow`, `a_deterministic_run_gives_the_same_result_every_time` |
| | Tool-selection tests | Cases name the tools a good answer calls and must not call, counted only when a call actually ran; the `tool-selection` starter suite (written by `cortex-desktop eval`) is three of them | `an_eval_suite_runs_through_the_loop_and_is_graded`, `answers_are_graded_on_tools_and_facts`, `an_eval_counts_only_calls_that_ran` |
| 5 Traces | `query_traces(filter)`, `get_run(id)` | `query_traces`, `get_run` built-ins (`cortex-traces::runs`, `run_detail`) | `runs_can_be_queried_and_opened_with_what_they_used`, `the_model_lists_runs_and_opens_one` |
| | `failure_summary`, `usage_stats` | Built-ins of those names, called in a turn; `usage_stats` with `artifact` (kind:id) gives one skill's, tool's, page's or fact's loaded and used counts and score | `a_turn_is_traced_and_the_model_can_read_the_trace`, `a_run_is_traced_and_summed_without_its_content`, `usage_stats_can_be_read_for_one_artifact` |
| | Question, tools, arguments, results, errors, latency, tokens | The trace database holds tools (written tools' runs included, attributed to the turn that called them), errors (as ok or not), empty results (a signal), latency and tokens per run, on purpose without content; the question, arguments and results stay in the encrypted conversation and activity log, linked by conversation id | `a_run_is_traced_and_summed_without_its_content`, `implicit_signals_are_counted_and_unknown_kinds_ignored`, `a_written_tool_s_runs_are_in_its_turn_s_trace` |
| | Artifacts loaded versus used; user feedback | `run_artifacts` per run: tools offered and called, facts listed, pages recalled and read, skills listed and opened; ratings with their reason; implicit signals, the source a link pointed to included | `a_run_records_what_it_had_and_what_it_used`, `ratings_are_kept…`, `implicit_signals_are_counted…` |
| 6 Memory | `memory_read`, `memory_write`, `memory_search`, `memory_forget` | Facts in the prompt and `knowledge_read`; `remember`, `knowledge_write`; `memory_search`, `knowledge_search`; `forget` | `memory_tools_save_and_forget`, `many_facts_become_a_short_index…` |
| | Facts, episodes, procedures | Facts and pages; past chats (`search_chats`, `read_chat`); skills | `memory_tools_save_and_forget`, `pages_are_written_found_and_recalled_by_relevance`, `skills_load_by_name` |
| | Short index in context, bodies on demand | Past 40 facts: important and relevant ones plus a count; pages by title, recalled in full when relevant; skills by name | `many_facts_become_a_short_index…` |
| | Scopes; provenance | `Memory::scope` (`project:`, `agent:`), set from the chat, never by the model; conversation id on each fact | `many_facts_become_a_short_index_and_scopes_apply`, `the_model_cannot_choose_a_fact_s_scope` |
| 7 Version control | `commit`, `log`, `revert`, `diff` | `versions::keep`, `list`, `restore`, `diff`; restoring keeps the version it replaces, so a revert can be reverted | `changed_items_keep_what_they_replace`, `a_memory_change_can_be_reverted`, `a_version_diffs_against_the_one_after_it` |
| 8 Lifecycle | `score(artifact)` | `cortex-kernel::lifecycle::score`, shown by `chat_artifacts` | `scores_favour_what_is_used_and_fade_when_idle` |
| | `promote`, `deprecate`, `archive` | `promote_tool` (an approval restarts the idle clock); the daily lifecycle pass (also archiving candidates never approved), which judges a written tool by when it last ran, also when another tool called it; a call refused before it ran is not counted against it | `only_the_assistant_s_unused…`, `unused_assistant_tools_retire…`, `a_tool_in_use_is_kept`, `tools_call_tools_three_levels_deep_at_most`, `a_refused_call_is_not_counted_as_the_tool_failing`, `artifacts_keep_versions_status_and_use`, `a_fresh_approval_is_judged_on_its_own_calls` |
| | Monitor and roll back | Daily, judging only calls since the user last approved it: a script tool that fails its regression checks, or fails more this week than last (`lifecycle::regressed`), goes back to its last approved, signed version that passes (only a tool in use; never forward to a version the user set aside by restoring an earlier one from History, which sets aside every version approved after the one brought back (a draft, the approvals after it was written; a History entry that does not verify, every approval, and it brings back the last signed checks and none of its own dates), a rewrite keeping that list; a restored version, or a restored draft from its approval, is compared week over week only once it has a week of its own calls as the baseline (approving it again later does not restart that), not against the weeks of the versions it set aside; a rollback restarts the window it is judged on, so it is neither rolled back again nor demoted for the replaced version's failures), keeping only the access it has now (a revoke is never undone, by the monitor or by an earlier signed file put back); with none, to candidate | `a_tool_that_got_worse_than_last_week_is_flagged`, `a_tool_that_got_worse_goes_back_a_version_or_out`, `a_tool_that_breaks_goes_back_to_its_last_good_version`, `a_rollback_does_not_undo_a_revoke`, `a_forged_version_is_never_rolled_back_to`, `a_failing_tool_is_demoted`, `the_monitor_leaves_a_retired_tool_retired`, `a_version_restored_from_history_is_judged_on_its_own_calls`, `a_fresh_promotion_that_does_worse_goes_back`, `the_monitor_never_rolls_forward_to_a_version_the_user_set_aside`, `a_restored_version_is_not_judged_against_the_weeks_it_set_aside`, `restoring_sets_aside_every_later_version`, `restoring_a_draft_keeps_the_earlier_approved_version_to_roll_back_to`, `restoring_a_draft_sets_aside_the_approvals_after_it`, `a_restored_draft_once_approved_is_not_judged_against_the_weeks_it_set_aside`, `a_reapproval_long_after_a_restore_is_monitored` |
| | `consolidate(scope)` | `cortex-kernel::lifecycle::duplicates` finds facts that say the same thing within a scope (a fact and its negation never do); `memory::consolidate` drops the assistant's duplicates (versioned) in the daily pass, never a fact the user wrote; the weekly knowledge review proposes the rest | `facts_that_say_the_same_thing_in_the_same_scope_are_grouped`, `duplicate_facts_are_merged_and_the_old_list_kept` |
| 9 Context | `load(ref)`, `unload(ref)` | `find_tools`, `use_skill`, `read_tool_output`; `unload_tools` (frees the context; the tool stays callable) | `tools_load_and_unload_on_demand_and_the_budget_can_be_read`, `a_long_result_is_kept_and_read_back_in_parts` |
| | `compact(history)` | Before every step past 60% of the window, once per turn on failure. Where to cut and what the summary covers are tested in the kernel; the summary itself is a model call | `compaction_cuts_at_a_prompt_and_keeps_recent_turns`, `a_summary_starts_requests_after_what_it_covers`, `a_failed_summary_is_tried_once_per_turn` |
| | `budget_status()` | `context_status` built-in | `tools_load_and_unload_on_demand_and_the_budget_can_be_read` |
| | Fixed budget for the always-loaded part | The prompt assembler, a quarter of the window, for the prompt's sections; essential sections are kept whole even over it, and tool schemas are outside it | `over_budget_the_least_important_and_largest_go_first` |
| 10 World access | Search, fetch, docs; output is data | Web search, browser, MCP, OpenAPI; the exposure gate | `after_reading_mail_new_urls_and_outbound_tools_need_approval`, `tools_outside_the_registry_pass_the_same_gate_as_connectors` |
| 11 Sub-agents | `spawn(role, task, tools)`, `collect`; isolated contexts | `delegate`: each task names an agent (role) and may name its tools (`subagents::task_tools`); workers are engine turns in temporary chats, which can neither write memory nor tools; a tool a turn did not offer never runs, whatever the model calls; reports come back in order | `a_task_gets_only_the_tools_it_names`, `a_delegate_worker_runs_through_the_same_loop`, `reports_render_as_json_in_order`, `a_tool_that_was_not_offered_does_not_run` |
| | The author is not the only reviewer | Before the user decides on a `promote_tool`, `propose_change` or `save_skill` (which asks in every mode, with its own card even under an agent's ask-first policy), a reviewer reads it: the Reviewer model from Settings (a different model), else the chat's model in a fresh request holding only the review; its verdict is on the card | `a_promotion_shows_the_independent_review`, `a_proposed_change_is_reviewed_and_applied_only_when_approved`, `without_a_reviewer_model_the_chat_s_model_reviews_afresh`, `a_declined_skill_leaves_nothing_behind`, `saving_a_skill_asks_even_in_auto`, `an_ask_first_agent_still_shows_the_skill_s_review` |
| 12 Human channel | `ask`, `propose(change, evidence)`, `request_approval` | `ask_user` (the turn waits for the answer), `propose_change`, approval cards | `ask_user_waits_for_the_answer_and_returns_it`, `a_change_asks_in_ask_mode…`, `a_proposed_change…` |
| 13 Scheduler | `schedule(cron, task)`, `on_event` | `schedule_task` with once, daily, weekly, monthly, yearly or a five-field cron expression in local time (also "Custom (cron)" in the task form); folder and feed triggers. Daily, weekly and monthly times go through the app's timing engine; cron expressions are computed with the same `croner` library directly, and yearly dates by hand | `cron_tasks_run_on_their_expression`, `tasks_and_schedules_share_one_timing_engine`, `new_files_start_a_run_once_settled`, `rss_and_atom_feeds_are_read` |
| | Nightly evals, regression checks, re-indexing, consolidation, demotions | Each tick the scheduler loop calls `tasks::maintenance` (the lifecycle pass and regression checks daily, the eval suites nightly and the review, with the week's failures, weekly when on) and `tasks::dispatch`, which starts the evals (`evals::nightly_and_notify`, which announces a dropped score) and the review. Library re-sync runs from the same loop | `maintenance_runs_each_job_when_it_is_due`, `the_scheduler_starts_what_is_due`, `a_nightly_drop_is_announced`, `nightly_evals_compare…`, `it_runs_once_a_day`, `connector_documents_are_mirrored_and_kept_in_step` |
| 14 Permissions | `request_capability(scope, duration, reason)`, `revoke` | `request_capability` (always asks, on a card that shows what the tool has now and what lapsed; adds to it, each grant with its own lapse, and only if the tool is unchanged since the card); Revoke access on Settings' list of what the assistant made (`chat_revoke_tool`); lapsed grants drop | `a_tool_s_access_and_retirement_go_through_the_user`, `a_capability_request_adds_to_what_the_tool_has`, `granted_access_lapses_and_can_be_revoked`, `a_short_request_does_not_shorten_a_lasting_grant_of_the_same_thing` |
| | Scoped credentials, keys never seen, identity from the platform | Keychain and vault: an agent's secrets reach the processes its tools start as environment variables; a written tool is granted `host=SECRET` credentials at promotion, and the host attaches the secret to that host's requests, so the script never sees it. Values are never shown to the model: masked in every tool's output and errors (literal matches, so an encoded copy is not caught), and key- and token-shaped text and labelled passwords are masked in a written tool's output. Scope and project ids are filled by the app, not the model | `agents_get_only_their_secrets`, `values_are_masked_in_output`, `a_credential_goes_only_to_its_host`, `secrets_in_tool_output_are_masked`, `secrets_in_a_tool_s_output_are_masked`, `the_model_cannot_choose_a_fact_s_scope`, `secrets_in_a_tool_s_error_are_masked` |

### The other sections

| Section | Where |
|---|---|
| 5 Evolution loop | Section 6a below, every step built |
| 6 Layers of adaptation | `propose_change` (memory, skill, instruction), `create_tool`, the prompt; weights are out of scope (section 8) |
| 7 Signals, attribution, two filters, traps | Evals, ratings, implicit signals, tool errors, cost; `run_artifacts` and `ablate`; the write-time filter in `remember` and `knowledge_write`, the read-time cut-off in retrieval; criticality apart from use |
| 8 Keeping the context flat | Section 6a below |
| 9 Guardrails | Section 6a below; the protected folders and the shell guard are tested |
| 10.13 The example's open points | Section 7 below: every point closed, each with its test |
| 11–12 Build order, minimum kernel | Section 6a below |

### Where the build differs from the first plan, and why

- **The loop is in `cortex-kernel`; each step's work is in the app.** `cortex_kernel::agent_loop` holds
  the loop's control flow (steps, the last step, guidance, cancelling) and is tested on its own; the
  app's `TurnSteps` implements one step: stream a model call, run the tools it called. The step's work
  stays in the app because it calls about 40 app modules, and a crate cannot depend back on the app
  (phase 3).
- **Python and Bash custom tools keep bubblewrap.** WebAssembly runs JavaScript here; plain JavaScript
  custom tools now run in it (`custom_tools::runs_in_wasm`), on every system.

### How each kernel tool stands now

| # | Kernel tool | Where it lives |
|---|---|---|
| 1 | Code sandbox | `cortex-sandbox` (WASM, fuel, memory, time, capabilities) for written tools; bubblewrap with a memory cap for `run_code` |
| 2 | Persistent storage | Encrypted files under `chat/`; `evals/` beside it, out of tools' reach |
| 3 | Tool registry | `ToolRegistry` for connectors, `engine/handlers.rs` for the engine's own, `script_tools` with candidate/active/deprecated, artifact rows with version, hash and provenance |
| 4 | Evaluation harness | `chat/evals.rs`: suites, grading, runs, `compare`, `ablate`, regressions; `cortex-desktop eval <suite>` |
| 5 | Trace introspection | `cortex-traces`; the model reads it with `usage_stats` and `failure_summary` |
| 6 | Memory | Facts, pages, notebooks, skills; every change versioned |
| 7 | Version control | `versions.rs` for agents, workflows, tools, memories, pages, notebooks, instructions; skill and canvas history as before |
| 8 | Lifecycle manager | `chat/lifecycle.rs`, daily: deprecate unused, archive, roll back or demote failing tools, drop the assistant's duplicate facts; the user's own are never touched, and critical ones are never retired for disuse (a failing one is still rolled back) |
| 9 | Context manager | Compaction every step, retrieval-ranked skills and tools, one persistent long-output store |
| 10 | World access | As before |
| 11 | Sub-agents | `delegate` workers are engine turns with the same gates; the reviewer is the Reviewer model, or a fresh, context-free call |
| 12 | Human channel | `ask_user`, approvals, `promote_tool` and `propose_change`, which always ask |
| 13 | Scheduler and triggers | One timing engine for native tasks and the web app's schedules; the daily lifecycle pass and opt-in nightly evals |
| 14 | Permissions and secrets | One gate for connector, OpenAPI, custom, computer and script tools; capability manifests for script tools; vault unchanged |

Measured on the code, after phase 0:

| Claim | Now |
|---|---|
| Agent loops | 4 (`engine`, `subagents`, `headless`, `terminal_chat`); only `engine` runs privacy, audit, hooks or vault |
| `engine::turn` | 736 lines |
| `Runner::run_inner` | 319 lines, 17 `if name == …` branches |
| Tauri commands in `commands.rs` | 276 |
| Traits in `chat/` | 0 |
| BM25 implementations | 2 (`libraries.rs`, `knowledge.rs`) |
| Version stores | 3 (`versions.rs`, skill history, canvas versions) |
| Schedulers | 2 (`chat/tasks.rs`, `schedules/`) |

Changes to the plan, from what phase 0 found:

1. **Build the test seam first.** Phase 1 rewrites `run_inner`, which lives inside `turn()`, and nothing
   tests a turn. So phase 1 now starts with phase 2's first two steps: the `Host` trait and the turn tests.
2. **Close the sub-agent gap early.** Done: see phase 0.
3. **Pull a minimal eval harness forward.** The test `Host` and scripted model the turn tests need are most
   of an eval runner. A handful of tool-selection cases can ride on them right after the seam, instead of
   waiting for phase 7, since learned skills and memory already change behaviour today.

## 2. What the kernel document asks for

Sections 2 to 4 are the baseline: Cortex as it was at commit `39318b4`, before this work. What stands
now is in "Checked against the kernel document" above.

Fourteen domain-independent tools in three tiers (create, improve, operate) and three rules:

1. Growth happens on disk; the context holds only pointers.
2. Nothing is "better" until it is measured against a test set the agent does not control.
3. Every self-modification is versioned, attributed and reversible.

Cortex follows rule 1 well, rule 3 in part, and rule 2 not at all.

## 3. Cortex against the 14 kernel tools

| # | Kernel tool | Cortex today | Gap |
|---|---|---|---|
| 1 | Code sandbox | **Partial.** `run_code` uses bubblewrap with a fresh process per call (`chat/sandbox.rs:295`); optional Podman workbench. | Time limit only: no CPU or memory limit. On macOS, Windows, or without bwrap it runs on the host. The bash connector and MCP stdio servers always run on the host. No WebAssembly. |
| 2 | Persistent storage | **Yes.** Files under `<config>/chat/`, encrypted at rest, folder sync. | Whole-file JSON, rewritten on every change and scanned linearly. No areas for tools or evals. |
| 3 | Tool registry | **Partial.** `ToolRegistry` holds connectors in memory (`tools/registry.rs:149`); policies and MCP fingerprints persist. | No per-tool record: no version, status, provenance or search index. Built-ins, OpenAPI, custom tools and computer use are outside it. |
| 4 | Evaluation harness | **No.** 657 unit tests, live model tests and 14 e2e specs exist, but none scores answers or tool choice. | Everything. Ratings and arena Elo are stored but never aggregated. |
| 5 | Trace introspection | **Partial.** Monthly audit log with tool, short arguments, outcome and milliseconds (`chat/audit.rs`). | No per-call tokens, latency or cost; usage is one total per conversation. The model cannot read any of it. |
| 6 | Memory | **Yes, the strongest area.** Facts, knowledge pages, per-agent notebook, skills listed by one line and loaded on demand. | Three separate implementations. Scope is global or per agent only. Provenance is uneven. No history. |
| 7 | Version control | **Partial.** | Three different mechanisms (`versions.rs`, skill `history/`, canvas versions). None for memories, knowledge, notebooks or instructions, which the model writes. |
| 8 | Lifecycle manager | **Mostly no.** Learned skills are saved disabled; `versions.rs` has a "production" pin; a weekly review proposes changes. | No usage tracking, no scoring, no deprecation or archive. |
| 9 | Context manager | **Yes.** Compaction at 60% of the window, old tool results clipped, deferred tools, context view. | Compaction runs once before the step loop (`engine.rs:1492`), so a 24-step turn can overflow. Two long-output stores, one lost on restart. |
| 10 | World access | **Yes.** Web search, fetch, browser, MCP, OpenAPI. | None structural. |
| 11 | Sub-agents | **Yes.** `delegate` (parallel, sequential, debate) and `message_agent`. | Workers run in a separate loop with no hooks, audit or compaction. No reviewer role for self-made changes. |
| 12 | Human channel | **Yes.** `ask_user`, six approval modes, approval cards. | No "propose a change with evidence" flow. The gate is spread over several layers. |
| 13 | Scheduler | **Yes, twice** (`chat/tasks.rs` and `schedules/`). | No maintenance jobs beyond re-sync and the weekly review. |
| 14 | Permissions and secrets | **Partial.** Keychain, vault with output masking, and an exposure tracker for prompt injection that goes beyond the document. | No capability manifest per tool. Vault secrets are env vars that sandboxed code can read. Three tool kinds skip policy and exposure checks. |

## 4. What blocks scale and reuse today

These are the structural problems, in order of cost.

**Four agent loops.** `engine::turn` (`engine.rs:1170`), `subagents::run_one` (`subagents.rs:197`),
`headless.rs:257` and `terminal_chat.rs:305` each run their own step loop. Hooks, audit, privacy filtering,
vault masking, compaction and model failover exist only in the first. The local API server makes raw model
calls with no tools.

This was a safety gap, not only duplication, and is now fixed for `delegate` workers (see phase 0). Workers
called connector tools and built-ins with no audit entry, no `before_tool` hook, no vault, and no privacy
filter on their requests. A user's blocking hook could be sidestepped by delegating. And the main loop puts
the real personal data back into the `delegate` call's task before it runs, so with the filter on, the
worker sent that task to a cloud model unfiltered. Headless mode, terminal chat and the API server still
lack these layers until phase 2.

**No boundary between engine and UI.** The engine takes `&AppHandle` everywhere; 53 files in the crate
mention it, and `chat/` declares no traits. This is why the CLI modes had to copy the loop, and why no unit
test can run a turn.

**Two tool models and six side paths.** Connectors use `Connector` and `ToolDefinition`; chat flattens
everything into `ChatTool` (`chat/tools.rs:53`). OpenAPI, custom tools, computer use and built-ins each
dispatch on their own. About 20 tools are matched by name in a 300-line `if name == X` chain
(`engine.rs:2069`). Adding one built-in touches three or four places.

**Authorization is not one gate.** Registry policy, exposure, approval mode, inline per-tool checks, agent
policy and hooks are checked in different places. Two consequences, both verified:

- Every built-in is declared `read_only: true` with no connector (`builtins.rs:32`), and `needs_approval`
  returns early for those (`chat/tools.rs:269`). `remember`, `save_skill` and `schedule_task` never ask.
- OpenAPI, custom tools and computer use return from `execute` before the registry dispatch
  (`chat/tools.rs:425-475`), so per-tool policy and exposure tracking do not apply to them.

**One crate.** 77 flat `chat::` modules, 276 Tauri commands in one 4,839-line file, no compile-time
boundaries, and a 32 GB target folder.

**Repeated concepts.** Retrieval is written four times (BM25 in libraries and again in knowledge, substring
search for chats, prefix scoring for tools). Versioning three times. Two schedulers. Two skill systems.

## 5. Target architecture

As built (phase 3 explains the change from the first sketch below): `crates/cortex-kernel` (the agent
loop and its decisions), `cortex-retrieval`, `cortex-traces`, `cortex-sandbox`, and the app, whose
`TurnSteps` does each step's I/O and whose `chat/host.rs` seam connects it to a window, a terminal, a
worker or a test. The first sketch:

```text
src-tauri/                     Cargo workspace
├── crates/
│   ├── cortex-kernel          no Tauri: loop, Tool trait, pipeline, Host trait, context, prompt
│   ├── cortex-store           files as source of truth, SQLite as index: artifacts, versions, traces
│   ├── cortex-retrieval       one retriever: BM25 + embeddings + rank fusion
│   ├── cortex-sandbox         one Sandbox trait: wasmtime, bubblewrap, Podman
│   ├── cortex-tools           built-ins and connectors (filesystem, MCP, OpenAPI, browser, ...)
│   └── cortex-evals           suites, runs, compare, ablate
├── app                        Tauri shell: commands, webview, tray; implements Host
└── cli                        headless, terminal chat, ACP, bots; implement Host
```

Four contracts hold it together.

```rust
/// One model for every tool: built-in, connector, MCP, OpenAPI, custom, WASM.
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;
    async fn call(&self, ctx: &CallCtx, args: Value) -> ToolResult;
}

pub struct ToolSpec {
    pub name: ToolName,          // namespaced: source/name
    pub description: String,
    pub parameters: Value,       // JSON Schema
    pub effects: Effects,        // read, write, destructive, open-world, executes code
    pub source: Source,          // Builtin, Connector, Mcp, OpenApi, Custom, Wasm
    pub version: u32,
    pub hash: Option<String>,    // content hash, pinned at approval
    pub status: Status,          // Candidate, Active, Deprecated, Archived
}

/// What a front end provides. The engine knows nothing else about the UI.
pub trait Host: Send + Sync {
    fn emit(&self, event: ChatEvent);
    async fn approve(&self, request: ApprovalRequest) -> Decision;
    async fn ask(&self, question: Question) -> Answer;
}

/// One ordered pipeline that every tool call passes through.
pub trait Stage: Send + Sync {
    async fn before(&self, call: &mut Call) -> Verdict;      // Proceed, Ask, Refuse
    async fn after(&self, call: &Call, result: &mut ToolResult);
}
```

The pipeline order is fixed: user hooks, tool and agent policy, exposure, approval mode and risk, secret
injection, execution, secret masking, result store, trace. A tool cannot opt out of a stage; it can only
declare `effects`, and the stages decide.

### Own loop or rig's agent runner

Cortex depends on `rig-core` only. The APIs the kernel example uses (`AgentBuilder`, `AgentHook`,
`ToolServer`, `add_dynamic_tool`, `retrieved_tools`) live in the separate `rig-agent` crate, which is not a
dependency today. `DynamicTool` and `ToolAnnotations` are in `rig-core`.

Recommendation: keep Cortex's own loop and unify it behind the contracts above. It already does things the
example does not: steering mid-turn, failover to a backup model, withholding tools on the last step,
serial handling of calls that wait on the user. The kernel document itself warns that rig's tool and hook
APIs changed between 0.43 and 0.46. Owning the traits keeps that churn in one adapter. Revisit `rig-agent`
after phase 2, when the loop is small enough to compare fairly.

## 6. Roadmap

Each phase leaves the app shippable. Sizes are relative: S is days, M is one to two weeks, L is several
weeks.

### Phase 0. Quick fixes (S) — done on branch `claude/agent-kernel-phase-0`

- Built-ins declare an effect (`builtins::Effect`: reads, memory, standing). `schedule_task` and
  `save_skill` now follow the approval mode through the same gate as `manage_agents`: they ask in Ask
  and Accept edits, are refused in Plan and unattended runs, and run in Full auto. Memory writes
  (`remember`, `forget`, `knowledge_write`) still never ask; the user's switch in Settings governs them.
- OpenAPI operations, custom tools and computer use go through the registry's per-tool policy and
  untrusted-content gate (`ToolRegistry::dispatch_external`), and their results count toward the
  conversation's exposure. A screenshot is classed as sending nothing.
- Compaction is checked before every step, and a failed summary is not retried within the turn.
- The bubblewrap runner (and the unsandboxed fallback on Linux) caps a run's private memory at half of
  physical RAM with `RLIMIT_DATA`, and turns off core dumps.
- `delegate` workers now run each tool call through `engine::guarded_call`, the wrapper the main loop
  uses: `before_tool` and `after_tool` hooks, the agent's vault secrets (masked in results), and an
  activity-log entry. On a cloud model with the privacy filter on, their requests carry placeholders and
  their replies and tool calls are restored, as in the main loop.

Done later:

- **Process limit.** `RLIMIT_NPROC` counts every process the user runs, so it could not be used. Instead,
  where a systemd user session can hold it, each bubblewrap run starts in a transient scope with
  `TasksMax=256` (`sandbox::task_scope`), passing `systemd-run` the session bus and unsetting it inside.
  Where there is no user session (macOS, Windows, some containers), a run keeps its PID namespace and
  time limit; the Podman workbench has `--pids-limit`.
- **Characterization tests for `engine::turn`.** The engine takes Tauri's `AppHandle<Wry>`, and Tauri's
  mock app uses another runtime type, so a turn cannot run in a unit test. These tests move to the start
  of phase 1: introduce the `Host` trait, then write them, then change the tool dispatch.

### Phase 1. One tool model, one pipeline (M) — done

What was built:

- `chat/host.rs`: the `Host` trait (data folder, events, chat state, tool registry, model connection,
  approvals, notifications, and the Tauri app for what is not behind it yet), `TauriHost` for the
  window, and a `TestHost` with scripted models. `run_turn`, `turn`, the runner, `tools::execute` and
  sub-agents take a `Host` instead of `AppHandle`.
- `chat/engine/turn_tests.rs`: whole turns against rig's scripted model: a plain answer, a tool call, a
  declined approval, Plan mode, steering, failover to the backup model, a provider error, compaction
  failing once per turn, and a `delegate` worker.
- `chat/engine/handlers.rs`: the engine's own tools in a table by name. `Runner::run_inner` went from
  319 lines and 17 `if name == …` branches to 75 lines and none. `is_builtin` is gone; `builtins::NAMES`
  is the list.
- Chats take connector results whole (`CallContext::whole`) and keep long ones only in the persistent
  `chat/tool-outputs/` store; the volatile `tool_results__read` is no longer offered in chats. The web
  app's bridge still uses the result store.

Left for later, on purpose: `ChatTool` stays as the description of an offered tool (20+ modules build
one), and `Effects` exists for built-ins only. Replacing them is a rename across the crate with no
behaviour change, best done during the workspace split.

The original list:

- First, the seam: define `Host`, implement it for Tauri, and write the turn tests with a test `Host` and
  a scripted model (moved here from phase 2).
- Introduce `Tool`, `ToolSpec` and the staged pipeline inside the current crate.
- Wrap each existing kind in an adapter: `Connector` tools, MCP, OpenAPI, custom tools, computer.
- Move the ~20 name-matched branches out of `Runner::run_inner` into registered tools, a few per change.
- Delete `ChatTool`, `is_builtin` and the per-tool mode checks once nothing uses them.
- Merge the two long-output stores into one that persists.

Done when: the engine dispatches by registry lookup only, and adding a built-in means adding one file.

### Phase 2. One loop behind a `Host` (L) — done

What was built:

- `TurnRequest` gained `max_steps`, `only_tools` and `system`, so a run with a narrow job uses the same
  loop.
- Headless mode (`headless.rs`) runs `engine::turn` with a `HeadlessHost`: unattended (so the engine's
  own gates refuse changes), only the reading tools, in a temporary conversation deleted after. The host
  stops the turn at the token, price and time limits and builds `export.json` from the turn's events.
- Terminal chat (`terminal_chat.rs`) runs `engine::turn` with a `TerminalHost` that prints the stream.
- `delegate` workers (`subagents.rs`) run `engine::turn` with a `WorkerHost` that relays progress to the
  parent chat: a temporary conversation per worker, the brief as the whole system prompt, only the
  reading tools. Hooks, vault, activity log, privacy filter and the backup model now apply to workers
  because they are the engine's.
- There is one step loop (`engine.rs`, `for step in 0..max_steps`). The local API server's model calls
  proxy other apps' requests, which run their own loops; the rest are one-shot errands (titles,
  compaction, Auto's check).

- The prompt assembler (`cortex-kernel::prompt`): the system prompt is named sections (identity,
  instructions, project, tools' guidance, memories, skills, libraries, notebook, team, knowledge pages,
  plan mode…), each essential, useful or optional. Within a quarter of the model's window the optional
  ones give way first, largest first. Each turn saves the parts it sent (`Conversation::prompt_parts`),
  and the context view shows them as their own slices, with the remainder as tool definitions.

`turn()` still holds its stages in one function; with the prompt and every decision moved out, what is
left is the I/O sequence itself, covered by the turn tests.

The original list:

- Extend `Host` (built in phase 1) and the turn tests to every case: a plain answer, a tool call, an
  approval, a steer, a failover, a compaction.
- Split `turn()` into stages: load, select tools, assemble prompt, step loop, dispatch, post-turn.
- Replace the ten string appends with a prompt assembler that takes named sections, each with a token
  budget and a priority, so the context view reads the same structure the model receives.
- Move sub-agents, headless, terminal chat and the API server onto the same loop with their own `Host`.

Done when: there is one step loop in the code base, and a sub-agent's tool calls appear in the audit log.

### Phase 3. Workspace split (M) — done, as functional core and imperative shell

**The decision.** The plan's first idea was a `cortex-kernel` holding the whole loop. Measured, the loop
calls about 40 app modules directly (store, memory, skills, agents, the browser, computer use, the ACP
and coding-agent back ends, MCP connectors), 23 of the 88 `chat/` files and 14 of the `tools/` files use
Tauri, and a crate cannot depend back on the app. Moving the loop would mean moving most of the app.
Instead the kernel holds the loop itself and every decision it makes, with no window, files or
network, tested on their own: `agent_loop::run` decides how many steps a turn takes, that the last one
offers no tools, that guidance earns another step, and that cancelling stops at once. The app's
`TurnSteps` (`chat/engine.rs`) implements `agent_loop::Steps`: one model call streamed, then the tools it
called, run and saved. `Host` is the seam that
lets the shell run in a test, the terminal, a worker or the window.

`crates/cortex-kernel` (no Tauri, WebKitGTK or GTK in its dependency tree; `cargo test -p
cortex-kernel`):

- `agent_loop`: the loop, over a `Steps` trait the app implements.
- `approval`: approval modes, the calls that are hard to undo, which calls only read, Auto's check,
  and `change_gate` for changes outside the connectors.
- `tool`: `ChatTool` (a tool as offered to the model), `Outcome`, name and schema rules.
- `context`: what a request carries, clearing old results, compaction points, the context view.
- `prompt`: the prompt assembler.
- `privacy`: redaction and restoring.
- `evals`: cases, grading, comparing runs, ablation sets.
- `lifecycle`: what to deprecate, archive or demote, from the registry and a week of statistics.

The app re-exports each (`chat::context`, `chat::tools::needs_approval`, …), so callers did not change.

Three more crates hold the rest of the AI core that needs no window:

- `crates/cortex-retrieval`: the tokenizer and BM25 corpus.
- `crates/cortex-traces`: the SQLite trace store, feedback and artifact registry. It takes plain token and
  cost numbers; the app adapts rig's usage report.
- `crates/cortex-sandbox`: the WASM sandbox, its WIT contract and the interpreter component
  (`wasm/js-runtime/`, rebuilt by `scripts/build-js-runtime.sh`). The app passes a file reader, so sealed
  files are decrypted by the app, not the crate.

CI formats, lints and tests the whole workspace.

`commands.rs` (the IPC surface, 279 commands) is split by area into `chat/commands/*_commands.rs`:
conversations, agents, tasks, libraries, knowledge, canvases, tools, voice, code, providers, settings and
the rest. Shared helpers and types stay in `commands.rs`, which re-exports every area, so the app's
handler list and the webview are unchanged.

The original list:

- Turn `src-tauri` into a workspace and move the code into the crates in section 5, kernel first.
- `cortex-kernel` must compile without Tauri. Enforce it in CI.
- Split `commands.rs` by area as the modules move.

Done when: `cargo test -p cortex-kernel` runs without WebKitGTK installed.

### Phase 4. Traces and the artifact registry (M) — done

What was built (`cortex-traces`, wrapped by `chat/traces.rs`):

- `chat/traces.db` with `runs`, `model_calls`, `tool_calls`, `feedback` and `artifacts`. Measurements
  only: names, ids, times, tokens, cost, success. Arguments and results stay in the encrypted files,
  because this database is not sealed at rest.
- Every `engine::turn` is a run; each model call records its time, tokens, cost and failure kind; each
  tool call through `guarded_call` records its time and outcome.
- Ratings are recorded as feedback, per answer, with model and agent.
- Artifacts: tools count a use per call, skills a load when listed and a use when opened, skills and
  script tools a version (by content hash), status and provenance when saved.
- Built-ins `usage_stats` and `failure_summary` let the model read its own record.
- `check_budget` reads an agent's monthly cost from the trace, falling back to the old scan for a month
  with no traced run.
- `versions.rs` also keeps memories, knowledge pages, notebooks and instructions, so every change the
  assistant makes to them can be undone.

- Implicit signals: regenerating an answer, editing and resending a prompt, resuming a stopped turn
  (recorded by the engine), exporting a chat and copying an answer (by their commands) go to a
  `signals` table, and `usage_stats` reports them beside the ratings.

The original list:

- Add SQLite for derived and local data only: runs, steps, model calls (tokens, latency, cost), tool calls,
  feedback, and one `artifacts` table for tools, skills, memories, pages and agents with version, status,
  hash, provenance, and counts of times loaded and times used.
- Keep files as the source of truth for everything that syncs or is encrypted at rest.
- One versioning mechanism for every artifact, replacing the three in use, and extended to memories,
  knowledge pages, notebooks and instructions.
- Read-only tools for the agent: `query_traces`, `failure_summary`, `usage_stats`.
- Agent budgets read from the trace table and stop scanning every conversation.

Done when: "which tools failed most this week, and what did they cost" is one query, and the agent can ask
it.

### Phase 5. One retriever (M) — done

What was built (`cortex-retrieval`): one tokenizer and one BM25 `Corpus`, with optional prefix matching.

- Libraries and knowledge pages rank with it; their two copies of BM25 are gone.
- `find_tools` ranks with it (name weighted three times, prefixes on), replacing the word-count heuristic.
- Skills: past 30, the prompt lists the 15 that fit the user's message and says how to find the rest;
  `use_skill` with words instead of a name searches the skills.

Not changed on purpose: chat search (`index.rs`) keeps exact substring matching, which is what the
search box promises. Embeddings were first kept to the libraries; `find_tools` later fused a ranking by
meaning when an embedding model is set up, embedding each tool once (row 3 above).

The original list:

- Extract the library search (BM25, embeddings, rank fusion, optional reranker) into `cortex-retrieval`.
- Use it for knowledge pages, past chats, skills and tools.
- `find_tools` searches every tool kind with it, and skills stop being listed in full once there are many.

Done when: one index type backs all five, and the copies in `knowledge.rs`, `index.rs` and `deferred.rs`
are gone.

### Phase 6. WASM sandbox and agent-written tools (L) — done

What was built:

- `cortex-sandbox`: the example's host, adapted to a `script-tool` world (`run(script, args)`), with the
  gaps in section 7 closed. Response bodies are capped while streaming; host names are resolved, private
  addresses refused and the connection pinned (no DNS rebinding); the component compiles once per
  process; limits are fuel, a memory limiter and an epoch deadline plus a timeout.
- The interpreter (route B): `wasm/js-runtime/`, Boa JavaScript compiled to a 4 MB WASI component. A
  script defines `function main(args)` and reaches only `log`, `fetch`, `read`, `write` and `tool`, each capability-checked
  by the host. Arguments enter as a JSON string literal, never as code.
- `chat/script_tools/`: `create_tool` saves a candidate with no capabilities and test-runs it with none;
  `promote_tool` always asks, showing the script, the description the model will read and the access
  asked for; an active tool is offered as `script__<name>` in every chat, read-only when it has no
  network, and refused if its script no longer matches the approved hash. The app's own tool names
  cannot be taken.
- Script tools go through the same gate as connectors (`dispatch_external`); with network access they
  count as able to send data out.

Tested end to end in `turn_tests.rs`: written in one chat, approved, offered and run in another; declined
and never offered.

Custom tools: plain JavaScript ones (no Node modules, process or files) now run in this runtime, in
program mode, on every system (`custom_tools::runs_in_wasm`). Python, Bash and Node-dependent JavaScript
keep bubblewrap, now with a memory cap, because WebAssembly runs only the interpreter built here. A known
cost: the first sandbox call in a debug build waits about 15 s while the component compiles (release
builds compile it far faster).

Added after re-reading the kernel document: composition (`tool(name, args)` from a script, for the
tools its capabilities name, three levels deep), regression checks per tool, near-duplicate refusal,
`retire_tool`, `request_capability` with a duration and `chat_revoke_tool`, a rate limit per tool, and
a guest that yields its thread instead of holding it.

The original list:

- Bring the example's `sandbox` crate in as `cortex-sandbox`, with its WIT contract, capability checks,
  fuel, memory and time limits, and BLAKE3 hash pinning. Its 13 tests come with it.
- Fix the example's known gaps before use (section 7).
- Start with route B from the document: one audited component that embeds an interpreter, so the model
  supplies only a script. No compiler toolchain on the user's machine.
- New flow: the model calls `create_tool`; the tool is stored as a candidate with no capabilities; it runs
  only in test calls; the user sees its code, description and requested capabilities and approves; it
  becomes active, pinned to its hash.
- Custom tools move to this runtime, which gives macOS and Windows a real sandbox for them.
- `run_code` keeps bubblewrap and Podman: native Python with packages does not fit in WASM.

Done when: the model can write a tool in one chat, the user approves it, and it is found by `find_tools` in
another chat.

### Phase 7. Evals and lifecycle (L) — done

What was built:

- `chat/evals.rs`: suites in `<config>/evals/<suite>/cases/*.json`, outside `chat/`. A case lists the tools
  a good answer calls or avoids and the facts it states. Cases run through `engine::turn`, unattended, in
  temporary chats, and are graded; runs are saved. `compare` names the cases that moved; `ablate` reruns
  without a tool or skill (`TurnRequest::tools_off` and the new `skills_off`).
- A starter suite (`tool-selection`), and `cortex-desktop eval <suite> [--model] [--without tool:x]`.
- An answer the user rates down becomes a case in `regressions`, with the wrong answer in its note,
  waiting for the user to say what a right answer contains.
- Promotion is gated: with the user's suites, the tool is tried in a copy of the data folder before the
  approval card, which shows the scores; a tool that makes a suite worse is never offered.
- `chat/proposals.rs`: `propose_change` (memory, skill or instruction, with evidence). A fresh model call
  with none of the chat reviews it; the user sees the review and decides, in every mode.
- `chat/lifecycle.rs`, run daily by the task scheduler: assistant-made skills and script tools unused for
  30 days are deprecated, for 120 days archived, and a script tool failing more than half of at least 5
  calls in a week is demoted to candidate. The user's own and critical artifacts are never touched.
- Settings → Knowledge has a "What the assistant made" section (`chat_artifacts`,
  `chat_set_artifact_critical`): each skill and tool the assistant wrote or learned, its status and use,
  and a switch that marks it critical.
- Nightly evals, opt-in (Settings → Knowledge, "Run my eval suites daily", `Learning::nightly_evals`):
  once a day the suites with expectations run on the default model, each is compared with its last run,
  and a notification says when a score drops.
- One timing engine: native tasks (`chat/tasks.rs`) compute their next run with the `schedules` engine
  (time zones, daylight saving, cron) for once, daily, weekly and monthly runs. Yearly tasks, folder and
  feed triggers, and monthly runs on the 29th to 31st (which fall back to a short month's last day)
  keep their own rules, which that engine does not express. The two run loops stay apart because they
  serve different front ends: one hands runs to the hosted web app through its bridge, the other runs
  native tasks through `engine::turn`.

The original list:

- `cortex-evals`: a suite is a folder of cases (prompt, expected facts or expected tool calls, grader). It
  runs through the same kernel with a headless `Host`.
- Three suites to begin: tool selection, memory recall, and regression cases saved from chats the user
  rated down.
- `compare(run_a, run_b)` and `ablate(artifact, suite)`: rerun with one skill, memory or tool removed.
- Eval folders are outside every path the agent can write.
- Gate promotion on evals: a learned skill or a candidate tool becomes active only if the suite does not
  get worse.
- Merge the two schedulers, then add maintenance jobs: nightly evals, usage scoring, deprecation of unused
  artifacts, memory consolidation.

Done when: a prompt or skill change shows a before and after score before it is enabled.

As built: with suites, a tool, skill, instruction or memory change is tried and measured before the
approval card, and the card shows each suite's score before and after; a change that makes any suite
worse is never offered (`a_change_that_makes_the_evals_worse_is_never_offered`). Without suites there is
nothing to measure, and the user's approval stands alone.

## 6a. Coverage of the rest of the kernel document

Section 3 covers the 14 tools. This maps the document's other sections to where each lands.

### The evolution loop (document section 5)

| Step | In Cortex | Phase |
|---|---|---|
| 1. Act | The agent loop | Exists; one loop in 2 |
| 2. Observe | Run traces and ratings, readable by the agent (`query_traces`, `failure_summary`) | 4 |
| 3. Diagnose | The weekly review (`learning::review_task`) carries the week's `failure_summary` and asks the model to open failed runs and say what would fix them; the model can also call `failure_summary` any time | 4, 7 |
| 4. Propose | `propose_change` built-in: a change (memory, skill or instructions) with its evidence, shown as an approval card; tools are proposed with `create_tool` and `promote_tool` | 7 |
| 5. Test | Candidate tools run only in the WASM sandbox; suites run with and without the change (`compare`, `ablate`) | 6, 7 |
| 6. Review | The Reviewer model (or a fresh, context-free call) reads the tool or change before the card reaches the user; its verdict is on the card, so the author is never the only reviewer. A skill saved with `save_skill` is reviewed and measured the same way and asks in every mode | 7 |
| 7. Promote | `promote(artifact)`: new version, status `active`, hash pinned | 4, 6 |
| 8. Monitor | Daily, for script tools: this week's failure rate against last week's, and the regression checks; a tool that got worse goes back to its last approved, signed version that passes, else to candidate. Skills and memory have no failure rate to compare: their changes are measured when made, nightly evals catch later regressions and notify, and History reverts them | 7 |
| 9. Prune | Unused for 30 days → `deprecated` (hidden, restorable); 120 days → `archived` | 7 |

### Layers of adaptation (document section 6)

| Layer | In Cortex | Status |
|---|---|---|
| 1. Memory entry | `remember`, `knowledge_write`, agent notebook | Exists; history and provenance in 4 |
| 2. Skill | `save_skill`, learned skills | Exists, versioned; usage counts in 4 |
| 3. Tool | Custom tools (user only); `create_tool` for the model | 6 |
| 4. Prompt or configuration | Instructions, agent preambles; the prompt assembler makes them named, budgeted sections | 2 |
| 5. Weights | Not in scope (section 8) | — |

`propose_change` (phase 7) states its layer, and the review prompt asks whether a lower layer would do.

### Telling useful from not useful (document section 7)

| Signal | Source in Cortex | Phase |
|---|---|---|
| Eval result | `cortex-evals` suites | 7 |
| Expert validation | The user's thumbs down with its optional reason, kept with the conversation and beside the rating in the trace (`rating_reasons`) | Exists; queryable in 4 |
| Explicit feedback | Ratings, arena Elo | Exists; aggregated in 4 |
| Implicit behaviour | Regenerate, edit-and-resend, resume, copy, export, a link in the answer opened; logged as trace signals | 4 |
| Technical outcome | Tool errors, empty results, retries, failover | 4 |
| Cost | Tokens, latency and price per model call | 4 |

- **Attribution:** every run records which artifacts were loaded (skills listed, facts listed, pages
  recalled in full, tools offered) and which were used (skill opened, page read, tool called). Phase 4.
- **Two filters:** at write time, `remember` and `knowledge_write` refuse sensitive data while the switch
  is on (the default), and their descriptions ask whether a fact will still be true in a month; at read
  time, the retriever's relevance cut-off decides what comes into the prompt (phase 5).
- **Traps:** criticality is a field separate from usage, so a rarely used but important memory is never
  pruned; the trace tables and eval folders are not writable by any tool. The usage the lifecycle reads is
  visible to the agent (`usage_stats`) and comes from its own calls, so it could keep a tool alive by
  calling it; it cannot keep one that fails, which the monitor and regression checks judge. The
  eval suites stay hidden: a trial tells the model only whether a change held, never the scores or the
  cases, which go on the user's card.

### Keeping the context flat (document section 8)

| Mechanism | Cortex | Phase |
|---|---|---|
| Deferred tool loading | `find_tools` past 30 tools, connector tools only | Connector, OpenAPI, custom and written tools, retrieval-ranked, in 5; the fixed core (built-ins, files, shell, browser) stays loaded |
| Skills as files | One line each, body on demand | Retrieved instead of listed once there are many, in 5 |
| Memory as files | Facts always in the prompt (up to 200); pages retrieved | Facts retrieved past a budget, in 5 |
| Code as composition | `run_code` in a workspace | Exists |
| Sub-agents | `delegate` returns reports | Exists |
| References | Long results saved, read with `read_tool_output` | One persistent store in 1 |
| Compaction | Summary at 60%, every step | Done in 0 |

### Guardrails (document section 9)

| Guardrail | How | Phase |
|---|---|---|
| The agent cannot modify its evaluator, permissions or audit log | The filesystem connector refuses the eval folders, the trace database, the audit log, `connector_state.json` and approval settings (tested through `fs_write`); `run_code`, written tools and eval cases cannot reach them. Shell tools run on the host, so a command that names the app's data folder asks in every mode, Full auto included. A command that reaches it without naming it is still under the approval mode: this is a guard, not a wall | 4, 7 |
| Humans own the eval sets | Suites live outside the agent's writable paths; the user writes and edits cases in Settings and adds them from rated answers | 7 |
| Creating and testing is free; promotion needs approval | Candidate tools run only in test calls; `promote_tool`, `propose_change` and `save_skill` always ask, in every mode, and rewriting an approved tool (which takes it out of use) asks too (`rewriting_an_approved_tool_asks_first`) | 6, 7 |
| New tools start with no capabilities | `create_tool` stores an empty capability set; grants are asked for separately | 6 |
| Read and write tools are separate; writes confirm | Every tool says whether it only reads (`read_only`); the approval mode decides from it, so in Ask mode writes confirm and in Auto and Full auto, by the user's choice, routine ones do not. Memory writes (`remember`, `knowledge_write`) do not ask: they are versioned, filtered for sensitive data and behind a switch. A script tool's effect is the union over the tools it calls. Only what a turn offered runs, and a written tool's effect is read at each call, so access granted mid-turn counts at once (`access_granted_mid_turn_changes_how_the_tool_is_called`) | 0 (built-ins), 1 (all) |
| Tool output is data, not instructions | The exposure gate; tool logs never reach the model | Exists; WASM logs in 6 |

### Build order and minimum viable kernel (document sections 11 and 12)

The document's minimum is sandbox, storage, registry and evaluation harness. Cortex has storage; the plan
adds the registry in 1 and 4, the harness first in 1 (with the turn tests) and fully in 7, and the WASM
sandbox in 6. Section 1 explains why the order differs: unifying the tool model and loop first means each
new kernel piece covers everything once, instead of one path of several.

## 7. Notes on the kernel example before adopting it

The example is sound and its limits are stated honestly in its section 10.13. Each of them is closed in
`cortex-sandbox`, `script_tools` and `sandbox.rs`:

| 10.13 point | In Cortex | Test |
|---|---|---|
| DNS rebinding | Resolve, refuse private, link-local, reserved and multicast addresses (IPv4 inside IPv6 too), pin the connection to the checked address, and never go through a proxy, which would look the name up again | `listed_hosts_still_cannot_reach_private_addresses_or_plain_http`, `private_addresses_are_recognized`, `the_connection_goes_to_the_checked_address` |
| Worker starvation | The guest yields its thread every epoch tick: on one thread, other work finishes while it runs | `a_busy_tool_does_not_starve_the_runtime` |
| Host-call timeout | The call's wall clock covers waiting on the host | `a_host_call_that_never_returns_is_stopped` |
| Compilation cost | Compiled once and kept on disk by hash | `the_compiled_interpreter_is_kept_and_reused` |
| Write capabilities | `write_prefixes` for workspace writes (folders made one at a time, never through a link), `tools` for tool-to-tool calls, `credentials` for requests; all asked for at promotion or through `request_capability`, and writing makes the tool not read-only | `writes_stay_inside_the_granted_folders_of_the_workspace`, `a_tool_that_writes_is_granted_its_folder_and_changes_something`, `a_tool_calls_another_approved_tool`, `a_credential_goes_only_to_its_host` |
| Persistence, signed binaries | Files (encrypted when the user turns encryption on) and the trace database; each approved tool signed with a keychain key over its script, description, parameters, grants and what the monitor and History go by, with each tool's latest signature recorded in the keychain so an earlier signed state put back does not verify; the key is made once and read back on later starts, and a trial's copy is held to it too | `a_tool_changed_outside_the_app_is_refused`, `the_signing_key_is_made_once_and_kept_in_the_keychain`, `a_trial_copy_refuses_what_the_app_refuses` |
| Output handling | A rate limit per tool, size caps on reads, writes, responses and a script's result, vault masking, a secret scan | `calls_past_the_rate_limit_are_refused`, `reads_and_writes_are_capped`, `a_response_past_the_cap_is_refused_as_it_arrives`, `secrets_in_a_tool_s_output_are_masked`, `a_trial_copy_has_its_own_rate_limit` |
| Semantics | The sandbox bounds reach; behaviour is held by the independent review, the regression checks and eval-gated promotion | `a_promotion_shows_the_independent_review`, `regression_checks_hold_a_tool_to_what_it_did`, `a_promotion_is_gated_on_the_user_s_evals` |
| Non-WASM workloads | The Podman workbench or bubblewrap; unsandboxed native code only with the user | `code_runs_unsandboxed_only_with_the_user`, `unsandboxed_code_asks_even_in_auto` |
| Rig API stability | Cortex's own loop and `ChatTool`; `rig-core` pinned exactly (`=0.46.0`) in the app and the kernel, no `rig-agent` code | `rig_core_is_pinned_exactly`, `tools_run_between_model_calls_until_an_answer` |

What the tests do not reach, and how each part is verified instead:

- **The real OS keychain and a machine without a sandbox.** Making and reading the signing key is tested
  against an in-memory store (`the_signing_key_is_made_once_and_kept_in_the_keychain`), and a run with no
  sandbox is tested by marking a test's data folder unsandboxed (`unsandboxed_code_asks_even_in_auto`);
  the OS keychain itself and a machine without bubblewrap were not used, since tests do not touch the
  user's keychain and the development machine has bubblewrap.
- **A credential on a real request.** Which secret goes to which host is tested
  (`a_credential_goes_only_to_its_host`), and so is the request it builds; a real HTTPS request to
  a granted host was not made, since tests do not reach the network.
- **A real embedding model.** Ranking by meaning is tested with a stand-in embedder
  (`tools_are_ranked_by_meaning_from_their_embeddings`); the HTTP call to the user's embedding server is
  the one libraries already use.
- **The WASI context's fields.** It is built empty in code (`WasiCtxBuilder::new().build()`); the tests
  check that scripts reach nothing through it, not the context's fields, which wasmtime does not expose.
- **Tauri's task spawning.** What the scheduler decides each tick is tested (`maintenance`, `dispatch`);
  the two calls that hand the work to Tauri's runtime need the running app.

Points from the review of the example, each handled when it moved into Cortex:

- **Response size is checked after the download.** `http_request` reads the whole body, then compares it
  with the 1 MiB cap. An allowed host can still exhaust memory. Read in chunks and stop at the cap.
- **Tool names come from the tool.** `describe()` supplies the name, so a tool could call itself
  `remember`. Namespace by source and reject collisions at registration.
- **The description is untrusted text shown to the model.** The approval card shows it, and the approval
  hash and signature cover it with the parameters, so a description changed after approval is refused.
- **Read-only is inferred from network access.** Now from every capability that changes something: a
  tool is read-only only when neither it nor anything it calls can reach the network or write.
- **`Component::new` compiles on every load.** Compiled once and kept on disk by hash.
- **Dependency drift.** The example uses `reqwest` 0.12 and the `rig` facade; Cortex uses `reqwest` 0.13
  and `rig-core`. Port the adapter to Cortex's own `Tool` trait and none of the `rig-agent` code is needed.
- **Build cost.** `wasmtime` is a large dependency. Put it behind a Cargo feature until phase 6 ships.

## 8. What not to copy from the document

- **Fine-tuning** (adaptation layer 5). Out of scope for a desktop app that uses other people's models.
- **Team and global scopes.** Cortex is single-user. The scopes that matter are global, project and agent.
- **A hidden eval set.** In the document, humans own the test set and hide part of it from the agent. Here
  the human is the user, so the rule becomes: the whole eval folder is out of the agent's reach (it can
  neither read nor change it through its own tools), the user writes the cases, and saved bad answers
  feed them.
- **Git as the version store.** A versions table with diffs is enough and needs no git binary.

## 9. Decisions to make

| Decision | Recommendation |
|---|---|
| Own loop or `rig-agent` | Own loop, behind Cortex traits. Revisit after phase 2. |
| SQLite | Yes, for traces and the artifact index only. Synced and encrypted content stays in files. |
| Route for agent-written tools | Interpreter component first (route B). Compiled Rust tools later, if ever. |
| Phase 3 timing | After phases 1 and 2. Splitting crates first would freeze the current coupling into crate APIs. |

## 10. Risks

- **Phase 2 is the risky one.** `turn()` mixes eight responsibilities, and behaviour such as steering and
  failover is easy to break. The characterization tests at its start are what make it safe; do not skip
  them.
- **Sync conflicts.** Sync is last-writer-wins on whole files. More self-written artifacts mean more
  conflicts in `memories.json` and `knowledge.json`. One file per artifact, in phase 4, removes most of it.
- **Signatures rest on the keychain.** A written tool's signature, and the record of which signature is
  each tool's latest, are only as safe as the OS keychain they live in. Someone who can write both the
  data folder and the keychain (any program running as the user, on most systems) can forge either. The
  agent's own tools reach neither (see the guardrails); the shell guard is a guard, not a wall.
- **Self-modification without measurement.** `save_skill` and auto-learned skills already change behaviour
  with no evals. Until phase 7, keep learned artifacts disabled by default, as they are today.
