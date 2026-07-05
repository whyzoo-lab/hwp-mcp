# Code of Conduct

## Our Pledge

We as contributors and maintainers of **rhwp** — an open-source HWP/HWPX parser and viewer built in Rust and WebAssembly — pledge to make participation in our project and community a welcoming and respectful experience for everyone, regardless of background, experience level, or technical perspective.

## Our Standards

### Community Behavior

Examples of behavior that contributes to a positive environment:

- Using welcoming and inclusive language
- Respecting differing viewpoints and experiences
- Giving and gracefully accepting constructive feedback
- Focusing on what is best for the community and the project
- Showing empathy toward other community members

Examples of unacceptable behavior:

- Harassment, trolling, or insulting comments
- Personal or political attacks
- Publishing others' private information without permission
- Any other conduct that could reasonably be considered inappropriate

### Code Quality Standards

rhwp applies a **hyper-waterfall methodology** with strict quality gates. Contributors are expected to follow these technical standards:

**Cognitive Complexity**
- All functions: Cognitive Complexity ≤ 15
- Warning threshold: > 10
- Hard block for new code: > 25

**SOLID Principles**
- Single Responsibility: one module, one concern
- No function or file should mix parsing, rendering, and serialization concerns
- Trait-based abstraction for parsers, serializers, and editors — avoid concrete type coupling

**CQRS (Command Query Responsibility Segregation)**
- Methods that mutate state (`&mut self`) are **Commands**: `insert_*`, `delete_*`, `set_*`, `apply_*`
- Methods that read state (`&self`) are **Queries**: `get_*`, `render_*`, `export_*`, `has_*`
- Commands must not trigger unnecessary re-pagination; batch Commands should defer layout recomputation until export
- Queries must not mutate state — Rust's `&self` vs `&mut self` enforces this at compile time

**Code Review**
- PRs must include a clear description of what changed and why
- Breaking changes require prior discussion (Issue or Discussion)
- All existing tests must pass; new features require accompanying tests

**Contribution Flow**
```
Issue → Branch → Plan → Implementation → Report → Approval → Merge
```
Do not skip steps. Maintainer approval is required before proceeding to each next stage.

## Enforcement

Instances of abusive, harassing, or otherwise unacceptable behavior may be reported via:

- [GitHub Private Vulnerability Reporting](https://github.com/edwardkim/rhwp/security/advisories/new) (for security issues)
- Direct message to the maintainer via [GitHub profile](https://github.com/edwardkim)

All complaints will be reviewed and investigated promptly and fairly. The maintainer is obligated to maintain confidentiality with regard to the reporter of an incident.

## Attribution

This Code of Conduct is adapted from the [Contributor Covenant](https://www.contributor-covenant.org), version 2.1, with project-specific code quality standards added.
