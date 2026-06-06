# Tool smoke test report (2026-06-05)

## OK
- shell_command: created `test-tools-temp`, wrote files, listed dir
- list_mcp_resources / list_mcp_resource_templates: empty but callable
- get_goal: returns null goal
- web_search: executed
- browse_page: fetched example.com

## Failed / unavailable this turn
- apply_patch: aborted (both add and update)
- codex_app__load_workspace_dependencies: not in tool set
- codex_app__read_thread_terminal: not in tool set
- mcp__node_repl__js: not in tool set

## Not exercised (not requested / no local image)
- view_image, read_mcp_resource, create_goal, update_goal, request_user_input, automation/thread tools

## Files
- test-tools-temp/shell-write.txt
- test-tools-temp/REPORT.md

## Retest (same session)
- apply_patch: ABORTED again
- shell_command, list_mcp_resources, get_goal, web_search: OK
- mcp__node_repl__js, codex_app__*: still NOT AVAILABLE
- retest-ts.txt written
