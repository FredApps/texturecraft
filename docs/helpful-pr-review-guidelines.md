# My PR review standards

Use these preferences when reviewing and improving a PR. Follow repository requirements for implementation and validation, and my explicit task instructions for the scope and authorized actions.

## Review scope

- Read the PR description, relevant issue discussion, and existing review threads; compare the diff against the intended base
- Review-only requests call for findings; requests to fix or improve also authorize focused local edits
- A review may conclude that no changes are needed. If the PR meets the applicable standards and no actionable issues are found, leave it unchanged. Do not invent findings, add unnecessary tests, or make edits merely to produce a diff. Report what was checked and any remaining validation limits
- Apply all review standards across the entire affected method, including existing code and relevant tests
- Complete naming, spacing, and comment cleanup throughout it
- Make additional correctness or performance changes when supported by evidence and appropriate validation
- Separate substantial changes that address an independent problem, explaining why
- Apply the same scope to standalone functions and inspect relevant callers when verifying behavior
- Distinguish problems introduced by the PR from pre-existing problems, and avoid repeating findings already covered by an unresolved review thread

## Correctness

- Verify the claimed fix against the actual definitions, code paths, and callers before proposing a change
- Identify a concrete trigger, expected behavior, actual behavior, and practical impact for each suspected defect
- Look for missed edge cases, incomplete fixes, and regressions, including malformed input, numeric boundaries, and partial failures where relevant
- Check compatibility with affected public interfaces, saved data, and supported platforms
- Reproduce suspicious behavior locally when practical; distinguish a reproduction from a conclusion based on code inspection
- Make regression tests fail before the fix and pass afterward; verify that the earlier failure demonstrates the defect rather than a broken build or test setup
- Confirm tests exercise the intended path, especially fallbacks and error handling, and assert observable behavior

## Optimization and simplicity

- Prefer the smallest complete solution that is efficient and easy to understand
- Look for repeated calculations, expensive iterations, unnecessary allocations, and redundant data access
- For loops, batches, and streaming operations, trace allocations and buffer lifetimes across multiple iterations, including outputs retained from earlier iterations. Identify work that belongs once per operation rather than once per item, using the full selection when making retention decisions
- Consider caching, batching, and parallelism when measurements justify their complexity and memory cost
- Measure relevant runtime, construction cost, and memory tradeoffs using representative workloads
- Compare before and after with the same inputs, build settings, and environment; account for measurement variation and cold or warm state where relevant
- Check what the compiler already optimizes before replacing clear arithmetic with micro-optimizations
- Avoid speculative optimization, unnecessary dependencies, and abstractions that add more code than clarity

## Clarity, naming, and spacing

- Clean up the functions or methods being changed and their relevant tests within the scope above
- Use descriptive names that explain purpose; include units where useful
- Avoid cryptic abbreviations, misleading names, and unexplained production constants
- Keep control flow direct and share repeated logic when it makes the code simpler
- Use a single blank line to separate logical steps, such as input preparation, validation, computation, mutation, and result construction, when those steps are distinct
- Keep closely related statements together; avoid spacing that fragments a single operation or inserts a blank line after every declaration
- Put a blank line after a multiline early-exit guard, including `if` and `let ... else`, before the next statement
- Keep comments attached to the code they explain; put a blank line before a comment introducing a new step, with no blank line between the comment and that step
- Keep documentation and attributes attached to their declarations
- Separate function definitions with one blank line; avoid extra empty lines immediately inside braces or between every field, argument, or statement
- Group tests into preparation, action, and verification with blank lines where those phases are distinct; keep related assertions together
- Apply this spacing consistently throughout affected functions, methods, and tests, and follow the repository's formatter for mechanical formatting

## Source organization

- Keep each class, enum, or struct in its own source file, with its implementation beside it
- Spell out the full type name in the filename; use the language's filename convention, such as `mesh_curvature_settings.rs` for `MeshCurvatureSettings` in Rust
- Use singular folder names, such as `map`, `setting`, or `tracing`
- Group related types into focused modules and share helpers where useful without collecting unrelated types into one large file
- Apply these preferences to new types and source organization changes within scope; follow mandatory repository layout rules and leave unrelated existing structure alone

## Comments

- Use short, single-line comments; adjacent comment lines should each express a concise point
- Avoid block comments and wrapped prose in ordinary implementation comments
- Explain intent or a non-obvious constraint rather than narrating the code
- No trailing punctuation on ordinary code comments
- Preserve explanations of guards and invariants; remove stale or redundant comments without losing necessary rationale
- API documentation, required license headers, and examples may use their conventional multiline formatting and punctuation
- Apply these standards to both production code and tests

## Validation and reporting

- Run relevant tests and required repository checks with the project's expected toolchain
- For ordinary comments or spacing alone, verify formatting and that executable code is unchanged; validate documentation examples or tool directives if they changed
- Report the commands actually run and their results; identify checks not run and why
- Distinguish existing failures from new ones using a baseline check when practical; otherwise state that the cause is unverified
- For behavioral defects, report the location, trigger, expected and actual behavior, practical impact, and supporting evidence
- For naming, spacing, or organization findings, identify the location and the specific standard being missed
- Order findings by severity and distinguish confirmed problems, hypotheses needing verification, and optional improvements
- Give measured optimization results with workload, build, and environment details; do not describe an unmeasured change as faster
- For memory-related changes, verify allocation/resampling counts or peak live memory in representative multi-item cases. Final pixel equality and shared output buffers do not prove that temporary allocations are avoided
- State remaining limitations and coverage gaps; if no actionable problems were found, say so without implying complete correctness
- Before handing back edits, review the final diff for unintended behavior changes, unrelated edits, and claims unsupported by validation

## Review and commit workflow

- Preserve unrelated work and leave changes unstaged for my manual review unless I ask otherwise
- Stage only when requested
- Wait for my explicit commit command before committing
- Do not post reviews or comments without explicit instructions; a request to review, fix, or improve alone does not authorize these actions
- When requested, provide a simple ready-to-post Markdown summary outside the repository and exclude it from the commit

## Push safety

- Default to no pushes; permission to edit, stage, commit, test, review, or prepare a PR never implies permission to push
- Push only on my explicit instruction to push, within the authorized remote, destination branch or tag, and scope of changes; permission for one task does not authorize unrelated pushes
- Before an authorized push, verify the remote URL, destination ref, and complete outgoing commits and diff; stop and ask if the destination or included changes are ambiguous or exceed my authorization
- Run an authorized push as a standalone command with an explicit remote and destination ref; never bundle it into commit, validation, or cleanup commands
- Check helpers, aliases, scripts, and hooks for automatic pushes before running them when they may publish; IDE sync or publish actions and APIs that update remote Git refs require the same explicit authorization
- Do not use broad pushes such as `--all`, `--mirror`, or `--tags`, force pushes, remote ref deletion, or hook bypasses without separate explicit authorization for that operation
- Never bypass repository branch protection or push directly to a branch that the repository requires to change through pull requests
