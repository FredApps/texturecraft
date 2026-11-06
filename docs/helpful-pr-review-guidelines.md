# My PR review standards

Review and improve the PR using these preferences. Follow the repository's instructions and keep changes focused on the problem being solved.

## Correctness

- Verify the claimed fix against the actual code and its callers
- Look for missed edge cases, incomplete fixes, and regressions
- Reproduce suspicious behavior locally when practical
- Make regression tests fail before the fix and pass afterward
- Confirm tests exercise the intended path, especially fallbacks and error handling

## Optimization and simplicity

- Prefer the smallest complete solution that is efficient and easy to understand
- Look for repeated calculations, expensive iterations, unnecessary allocations, and redundant data access
- Consider caching, batching, and parallelism when measurements justify their complexity and memory cost
- Measure relevant runtime, construction cost, and memory tradeoffs using representative workloads
- Check what the compiler already optimizes before replacing clear arithmetic with micro-optimizations
- Avoid speculative optimization, unnecessary dependencies, and abstractions that add more code than clarity

## Clarity, naming, and spacing

- Clean up the methods being changed, including their tests
- Use descriptive names that explain purpose; include units where useful
- Avoid cryptic abbreviations, misleading names, and unexplained production constants
- Keep control flow direct and share repeated logic when it makes the code simpler
- Separate logical steps with sensible blank lines so code is easy to scan
- Put a blank line after a multiline `let ... else` guard before the next statement; never place the next `let` directly after its closing `};`
- Keep related statements together and follow the repository's formatter

## Source organization

- Keep each class, enum, or struct in its own source file, with its implementation beside it
- Spell out the full type name in the filename; use the language's filename convention, such as `mesh_curvature_settings.rs` for `MeshCurvatureSettings` in Rust
- Use singular folder names, such as `map`, `setting`, or `tracing`
- Group related types into focused modules and share helpers where useful without collecting unrelated types into one large file

## Comments

- Use short, single-line comments or concise adjacent comments
- Avoid block comments and prose wrapped across multiple comment lines
- Explain intent or a non-obvious constraint rather than narrating the code
- No trailing punctuation on ordinary code comments
- Avoid overexplaining and remove stale or redundant comments
- Apply these standards to both production code and tests

## Validation and reporting

- Run relevant tests and required repository checks with the project's expected toolchain
- For comments or spacing alone, verify formatting and that executable code is unchanged
- Report concrete findings with locations, reproduction details, and practical impact
- Separate confirmed problems from hypotheses and optional improvements
- Give measured optimization results with workload and environment details
- State remaining limitations and distinguish existing failures from new ones

## Review and commit workflow

- Preserve unrelated work and leave changes unstaged for my manual review unless I ask otherwise
- Stage only when requested
- Wait for my explicit commit command before committing
- Do not push or post reviews or comments without explicit instructions
- When requested, provide a simple ready-to-post Markdown summary outside the repository and exclude it from the commit

## Scope of method fixes

- Whenever a method needs a fix, apply all of my review standards across the entire method, including existing code outside the immediate fix
- Ensure the whole method has sensible spacing between logical steps and concise, useful comments that follow the standards above
