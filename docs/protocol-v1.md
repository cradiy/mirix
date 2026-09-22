# Mirix Protocol v1

English | [简体中文](protocol-v1.zh-CN.md) · [Basic usage](../README.md)

This contract covers daemon IPC and out-of-process input method communication.
The protocol version is independent of application, crate, and skin format versions.
Separately built programs can communicate if they implement v1.
The current sending version is `1`; supported versions are `[1]`.

## Framing

Every request, response, and event uses this envelope, not newline-delimited JSON:

| Offset | Size | Contents |
| --- | --- | --- |
| 0 | 4 bytes | UTF-8 JSON payload length, u32 little-endian |
| 4 | 2 bytes | Protocol version, u16 little-endian; `01 00` for v1 |
| 6 | Declared length | UTF-8 JSON payload |

The payload limit is `1,048,576` bytes, excluding the six-byte header.
Frames may arrive in fragments or consecutively. Validate the length and version before
parsing the payload; the blocking reader checks them before allocating and reading the payload.
An oversized frame or unsupported version terminates the exchange.
Never interpret an unsupported version as the current message format.
`UnsupportedVersion` reports the received and supported versions; a business-level
error response to an incompatible peer is not guaranteed.

Complete frame examples:

```text
Hello: 07 00 00 00 01 00 22 48 65 6c 6c 6f 22
Ping:  06 00 00 00 01 00 22 50 69 6e 67 22
```

Rust callers use `encode_frame` / `decode_frame` or `write_message` / `read_message`.
Both paths check the version. `decode_frame` returns `None` for incomplete supported frames,
or the decoded message and consumed byte count on success.
The SDK exports `PROTOCOL_VERSION`, `SUPPORTED_PROTOCOL_VERSIONS`, and `FRAME_HEADER_SIZE`.

## Compatibility

- v1 message names, field types, requiredness, enum representations, and semantics must remain compatible across builds.
- JSON object member order is irrelevant. Unknown object fields may be ignored; new enum variants or operations cannot be assumed to work on existing peers.
- Optional additions must preserve v1 behavior when absent. Correctness must not depend on the peer understanding them.
- Breaking changes require an explicitly introduced protocol version and version-specific handling, not changed semantics under v1.
- No negotiation or conversion for other versions is implemented. Unsupported versions are rejected explicitly.
- State `revision`, host `session`, and candidate IDs guard against stale operations; they are not protocol versions.

The Rust SDK source API is not a process ABI. Other languages may implement the same wire format.
Field definitions: [engine messages](../crates/mirix-engine-protocol/src/lib.rs),
[input state](../crates/mirix-engine-protocol/src/engine.rs),
[daemon messages](../crates/mirix-ipc/src/messages.rs).

## Engine communication

The installation directory name is the engine ID; its executable must be named `engine`.
The host sets the working directory to that directory and supplies `MIRIX_ENGINE_NAME`,
`MIRIX_CONFIG_DIR`, `MIRIX_DATA_DIR`, and `MIRIX_CACHE_DIR`.
The latter three resolve under the user's configuration, data, and cache roots as
`mirix/input-methods/<ID>`, `mirix/input-method-data/<ID>`, and `mirix/input-methods/<ID>`.
The engine creates and maintains its own files. Discovery does not execute an engine;
enabling it starts the process. Rust engines implement `Engine` and may use the SDK's `serve` loop.

Frames travel over the child's stdin/stdout; logs go to stderr.
Requests are serial. Each has one ordinary response, possibly preceded by `IconChanged`.
At startup, the host sends the JSON string `"Hello"`; the engine replies with
`IconChanged` followed by `Ready`. Messages with fields use external tags,
for example `{"Select":{"id":42}}`.

| Request | Ordinary response |
| --- | --- |
| `Hello` | `Ready` |
| `ProcessKey`, `Select` | `State` |
| `Reset` | `ResetDone` |
| `SetCandidatePages` | `PagesSet` |
| `SelectMenu` | `MenuState` |
| `GetConfiguration`, `Configure` | `Configuration` |
| `Vocabulary` | `VocabularyState` |

An operation failure returns `Error { message }`; it does not imply a broken transport.
Invalid response state, a timeout, or process exit invalidates the engine session without
publishing partial state. Writes and all response reads share a deadline:
2 seconds for the handshake, 250 milliseconds for input, and 10 seconds for configuration
and vocabulary operations. `serve` handles dispatch and validates outgoing state.

### Input state

`EngineOutput` contains the consumed flag, optional commit, and updates for application
and panel composition. Candidates and selection travel in the same response.
The tested [complete State example](../crates/mirix-engine-protocol/tests/fixtures/v1-state.json)
demonstrates independent application letters, panel reading, and candidates.

- `preedit` is `{"type":"keep"}`, `{"type":"clear"}`, or
  `{"type":"set","value":{"text":"ni","cursor":2,"selection":null}}`.
- `Composition.text` is nonempty. Cursor positions are UTF-8 byte offsets.
  Selections are nonempty, end-exclusive ranges such as `{"start":0,"end":2}`.
  Every position must lie on a character boundary.
- A commit must explicitly clear or replace application preedit; a commit and the next composition may coexist.
- `panel_composition` is `keep` or `set`; the setting is `follow_application`,
  `hidden`, or an independent `composition`. Independent content is not suppressed
  by rules hiding duplicate application preedit; the skin controls its styling.
- Focus loss, reset, and engine changes clear both compositions. The commit helper restores panel follow mode.

`Candidate` contains `id`, `text`, optional `comment`, optional `reading`, and `keep_open`.
IDs are opaque u64 values, unique within a snapshot. Selection uses IDs, not array indices.
`selected` is a zero-based index, or 0 for an empty list.
Replace candidate IDs when replacing the candidate set to reject stale choices.
Queries and no-op operations preserve IDs, selection, and pagination.

A nonempty `reading` is engine-controlled and takes display priority over ordinary comments;
the framework does not infer pronunciations.
`keep_open: true` keeps the panel visible while awaiting a selection result.
The engine's response determines the final state; the engine also handles keyboard selection.

`SetCandidatePages.ends` contains increasing, exclusive page ends.
The last end equals the candidate count, each page has at most nine items, and an empty list uses `[]`.
For example, `[3,8,12]` defines three pages. The daemon validates the context and candidate-ID
snapshot before forwarding boundaries. Candidate changes invalidate old boundaries.

### Configuration and extensions

Increment `configuration_revision` when configuration state changes.
Configuration events carry their source revision; stale writes are rejected.
Configuration, menu, and vocabulary operations may return the same `EngineOutput`,
independently deciding whether to change input. Success does not require a commit or candidate change.

**Settings:** `Configuration { title, description, groups }` describes a page.
Each group has `id, label, controls`; controls have `id, label, kind`, with optional
`description` (empty), `enabled` (true), and `tray` (false).
Group IDs are unique within the page; control IDs are unique across groups.
Display text follows the requested locale.

| kind.type | State |
| --- | --- |
| `switch`, `checkable` | `value: bool` |
| `checklist` | `values: string[]`, `choices` |
| `select` | `value: string`, `choices` |
| `path` | `value: string`, `target: "file" or "directory"` |
| `text` | `value`, `submit_label`; optional `placeholder`, `clear_on_submit` (false) |
| `action` | None |

choices is an ordered array of `{ value, label }`, with unique values and valid selections.
Events are `{"type":"change","id":"...","value":...}` or `{"type":"invoke","id":"..."}`.
The engine validates, saves, and returns the authoritative page. Success may include message;
failure returns Error. Text submits only on Enter or button click; clear_on_submit clears only
after success. Path controls pass paths, not file contents.
See [configuration.rs](../crates/mirix-engine-protocol/src/configuration.rs).

**Menus:** `EngineMenu { id, label, shortcut, entries, active }` uses `{ id, label }` entries.
Menu IDs and shortcuts are unique per engine; entry IDs are unique per menu.
active may be null; otherwise it must exist. The host handles navigation and sends SelectMenu
using opaque IDs. Framework shortcuts take precedence; stale popup-session selections are rejected.
See [menu.rs](../crates/mirix-engine-protocol/src/menu.rs).

**Icons:** `EngineIcon.images` supplies 1–4 distinct RGBA8 sizes, each dimension 1–128 pixels,
straight alpha, no padding, and at most 64 KiB of pixel data in total.
IconChanged precedes Ready for each Hello, then precedes ordinary responses only when the value changes.
null clears the icon. Idle push and icon animation are unsupported.
See [icon.rs](../crates/mirix-engine-protocol/src/icon.rs).

**Personal vocabulary:** Vocabulary supports List, Add, Remove, Forget, Pin, and Hide.
Text plus complete reading identifies a word. profile checks the current profile; it does not switch it.
Queries and no-ops preserve input; mutations may return input updates.
See [vocabulary.rs](../crates/mirix-engine-protocol/src/vocabulary.rs).

## Daemon IPC

The Unix socket defaults to `$XDG_RUNTIME_DIR/mirix/mirix.sock`, or
`/tmp/mirix/mirix.sock` without that variable; `--socket` overrides it.
Requests and responses remain ordered per connection; subscriptions also receive full snapshots.
Asynchronous management operations return `Pending`, then a final result on the same connection.

| Purpose | Main requests |
| --- | --- |
| Lifecycle | `Ping`, `Restart`, `Shutdown` |
| Input contexts | `CreateContext`, `DestroyContext`, `FocusIn`, `FocusOut`, `Key`, `Select`, `Reset`, `SetCaps` |
| Method management | `SubscribeInputMethods`, `RefreshInputMethods`, `SetInputMethodEnabled`, `ReorderInputMethods`, `RestartInputMethod`, `SetShortcut` |
| Engine operations | `ToggleInputMethod`, `SetInputMethod`, `GetEngineConfiguration`, `ConfigureEngine`, `SelectEngineMenu`, `Vocabulary` |
| Panel | `SubscribePanel`, `SetCandidatePages`, `EnableQuickSwitch`, `ChooseQuickSwitch`, `SetPanelSkins`, `UiPreferenceChanged` |
| Application permission | `GetImeState`, `SetImeEnabled` |

Management `request` values correlate results; `session` and `revision` prevent stale writes.
Panel subscriptions and selection responses use monotonically increasing snapshot revisions.
Discard old state and resubscribe after disconnecting. Do not automatically retry failed mutations.

This is a local user-session control interface, not a check that callers own an application window.

An application sends `"GetImeState"` to obtain context_id, then pauses with
`{"SetImeEnabled":{"id":7,"enabled":false}}`; true releases that connection's pause.
Replies report the combined ImeState; without focus it is `context_id: null, enabled: false`.
A pause applies only to the current focus interval and requires an open connection.
Disconnect, focus loss, or context destruction releases it. All connections must release
their pauses before input resumes; stale IDs are rejected.
The first pause cancels composition without committing. Paused keys bypass the engine;
resuming does not restore canceled input. Applications track their own focus and must not
forge FocusIn to control another window.
