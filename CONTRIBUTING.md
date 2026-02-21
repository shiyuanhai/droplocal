# Contributing to DropLocal

Thanks for contributing to DropLocal.

## Quick start

1. Fork the repo.
2. Create a feature branch from `main`.
3. Install dependencies:

```bash
npm install
```

4. Run tests before each push:

```bash
npm test
```

5. Open a pull request with a clear summary and testing notes.

## What to contribute

- Bug fixes
- UX improvements (especially mobile)
- Performance improvements
- Docs and examples
- Tests for edge cases and regressions

## Development guidelines

- Keep runtime dependencies minimal.
- Prefer Node built-ins where practical.
- Keep the UI lightweight and framework-free.
- Preserve mobile-first behavior.
- Include or update tests for behavior changes.
- Keep API and WebSocket behavior backward compatible for existing clients.

## Pull request checklist

- [ ] I ran `npm test` locally.
- [ ] I added/updated tests for new behavior.
- [ ] I updated docs (`README.md`, `HISTORY.md`) when needed.
- [ ] I verified mobile layout (narrow viewport).
- [ ] I kept changes focused and avoided unrelated edits.

## Reporting issues

When opening an issue, include:

- Environment (`node -v`, OS, browser/device)
- Steps to reproduce
- Expected behavior
- Actual behavior
- Screenshots or terminal output when useful

## Release notes

User-visible changes should be added to `HISTORY.md`.
