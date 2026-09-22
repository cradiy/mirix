# Development Guidelines

These instructions apply to the entire repository. The project is in active development. Implement current requirements directly without adding maintenance obligations for historical designs.

## Protocol Contract

- Engine transport and daemon IPC use the v1 wire contract in `docs/protocol-v1.md`.
- Preserve v1 message semantics and wire representations across independent builds. Check the frame version before decoding a payload and reject unsupported versions explicitly.
- Version-boundary tests and fixed v1 wire fixtures are allowed and should protect observable interoperability.
- Do not introduce another protocol version, negotiation, or conversion layer without an explicit user request. Do not disguise breaking wire changes as v1 additions.
- The restrictions below apply to historical designs and unrequested compatibility work, not to maintaining the adopted v1 contract.

## Work Prohibited by Default

Unless the user explicitly requests it:

1. **Do not preserve compatibility.**
   - Do not retain compatibility branches for old interfaces, fields, configuration formats, or behavior.
   - Do not add compatibility layers, aliases, migration logic, legacy fallbacks, or deprecation periods.
   - Replace superseded implementations directly instead of keeping old and new designs side by side.

2. **Do not add compatibility tests.**
   - Do not add tests for supporting legacy interfaces, fields, configuration formats, protocols, or behavior.
   - Do not write tests asserting that removed interfaces, fields, or protocols are rejected.
   - This restriction applies only to compatibility tests. Meaningful unit, integration, functional, and regression tests for the current implementation may be added, updated, and run as needed for the task.

3. **Do not upgrade protocol versions.**
   - Do not automatically bump protocol or format versions, or add version negotiation, version branches, or version compatibility mechanisms.
   - Changes to interfaces or fields during development do not authorize defining or publishing a new protocol version.

4. **Do not write or retain documentation for old protocols.**
   - Describe only the currently adopted protocols, interfaces, configuration, and behavior.
   - Do not add legacy protocol inventories, old-versus-new comparisons, migration guides, compatibility notes, or deprecation notices.
   - Update relevant documentation directly when implementations change; do not retain abandoned designs as supported alternatives.

## Documentation Language and Status

- Write all AI-facing documents and skills in English, including `AGENTS.md`, `SKILL.md`, agent instructions, and supporting instructions intended for AI use.
- Documents intended only for the user may use the user's preferred language; they do not need to be English.
- Keep documentation concise and describe implemented behavior. Do not retain discussion transcripts, completed-work journals, speculative roadmaps, or obsolete commands as current guidance.
- User-facing documentation should describe current usage, not justify changes. Omit dependency commit hashes, revision-pinning explanations, and commentary about removed behavior.
- Do not discuss the user's other projects' licenses in Mirix documentation. Keep Mirix's own license declaration separate from dependency descriptions.
- The v1 wire protocol is the interoperability contract. Internal Rust APIs, private engine storage, and skin files are separate concerns; do not imply that protocol v1 freezes them.
- Keep one authoritative explanation per topic and link to it instead of duplicating implementation details.
- Keep project documentation limited to basic usage (`README.md`), the wire contract (`docs/protocol-v1.md`), and the Pinyin user guide (`docs/pinyin.md`), with matching `.zh-CN.md` translations. Unsuffixed documents are English and are the default entry points. Keep both languages aligned and link directly to the corresponding translation.
- Keep these development instructions in `AGENTS.md`; do not add separate design notes or other topic guides unless explicitly requested.
- Documentation cleanup does not authorize deleting unrelated tests, assets, or user changes.

## Execution Boundaries

- Keep only the character dictionary in this repository. Phrase dictionaries belong in a separate configuration directory. Do not retain corpus-specific conversion tools, source-selection rules, or upstream vocabulary documentation, including relocated conversion tooling.

- A request to implement, fix, or refactor does not authorize the prohibited work above. Exceptions require an explicit user request.
- Compilation, formatting, static checks, and tests of the current implementation are allowed when relevant to the task; they do not require a separate user request.
- Report verification accurately. Do not claim tests passed or runtime acceptance was completed when those checks were not performed.
- Do not delete unrelated existing tests, documents, or user changes without authorization.
