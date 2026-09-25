# Pinyin guide

English | [简体中文](pinyin.zh-CN.md) · [Basic usage](../README.md)

The Pinyin engine is not ready to use out of the box. Only a character dictionary is bundled;
you need to [import your own phrase dictionaries](#add-pinyin-dictionaries) before daily use.

## Start typing

Enable Chinese Pinyin in Settings and switch to it with `Ctrl+Space`.
Type lowercase pinyin. Full spellings, initials, mixed full/abbreviated input,
and an incomplete final syllable are supported.
For example, `shurufa`, `srf`, and `shurf` can find words with the corresponding reading;
actual results depend on the loaded dictionaries.

| Key | Action |
| --- | --- |
| Space | Commit the selected candidate |
| Enter | Commit the raw input, not the selected candidate |
| 1–9 | Select an item on the current page |
| `-` / `=`, PageUp / PageDown | Previous / next page |
| Arrow keys, Home / End | Move candidate selection |
| Backspace | Delete input |
| Esc | Cancel composition |
| Ctrl+. | Toggle Chinese / ASCII punctuation without clearing input |
| Ctrl+Shift+P | Open profiles while candidates are visible; configurable in Pinyin settings |

Click a candidate to select it. Browsing pages does not record learning.
Switching input methods cancels uncommitted composition.

Type `v` for `ü`, as in `nv`. Use an apostrophe to mark syllable boundaries, as in `xi'an`.
When an apostrophe closes a segment with a valid full-syllable character match,
that segment stays a single syllable rather than being split internally.
Apostrophes remain part of the input and are not ignored by non-pinyin extensions.

## Settings

Open `mirix-panel --settings` and select Chinese Pinyin:

- **Profile:** choose a dictionary and learning environment.
- **Show candidate pinyin:** toggle pronunciation labels without changing matching or learning.
- **Punctuation:** choose Chinese or ASCII punctuation; Ctrl+. also toggles it.
- **Manage current profile:** create, rename, duplicate, or archive.
- **Profile menu:** change this engine's menu shortcut.
- **Reload dictionaries:** apply changes to dictionary files.
- **Personal vocabulary:** manage personal entries, learning, and preferences.

Candidate readings use lowercase letters with spaces between syllables, such as
`你好 [ni hao]` and `女 [nv]`. The display preference is stored as `show_reading`
in the Pinyin configuration directory's `config.toml`; normally use the UI.

In Chinese punctuation mode, punctuation commits the selected candidate together with the symbol.
During composition, apostrophes still delimit syllables and `-` / `=` still turn pages.
A dot immediately after a typed number passes through for decimal input.
The punctuation preference is shared across profiles.

## File locations

The default configuration root is `~/.config/mirix/input-methods/pinyin/`;
the data root is `~/.local/share/mirix/input-method-data/pinyin/`.
These follow XDG_CONFIG_HOME and XDG_DATA_HOME respectively.
The installed engine executable directory is not a personal-data directory.

```text
configuration/
├── config.toml
├── characters.tsv
├── dictionaries/                  # Shared phrases
├── extensions/                    # Shared extensions
│   ├── exact.tsv
│   └── completion.tsv
└── profiles/<profile>/
    ├── dictionaries/              # This profile only
    └── extensions/
        ├── exact.tsv
        └── completion.tsv

data/
├── active-profile.json
├── profile-shortcut.json
├── punctuation-mode.json
└── profiles/<profile>/
    ├── learning.json
    └── vocabulary.json
```

Startup creates a missing character dictionary and missing empty extension files without
overwriting existing content. Reinstalling the program does not require deleting these directories.
Back up both configuration and data; exit Mirix normally before copying learning data.

## Add pinyin dictionaries

### Characters

Each `characters.tsv` row has three whitespace-separated fields:

```text
# character input-key weight
闺 gui 38845
女 nv 30000
```

Use complete lowercase letter syllables (`v` for `ü`) and nonnegative integer weights.
Program updates do not overwrite an existing character file.

### Phrases

Create a UTF-8 `.tsv` file under `dictionaries/`.
Separate input key, complete display reading, output text, and optional weight with actual tabs:

```text
shurufa	shū rù fǎ	输入法	1000
chengxu	chéng xù	程序	800
```

The example contains real tabs between columns. Spaces inside display readings separate syllables,
not columns. After removing separators, the input key must match the display reading with tone marks
removed and `ü` changed to `v`. Provide complete syllables, not initials;
the engine handles abbreviation matching itself.

Character and phrase files allow blank lines and comments starting with `#`.
Phrase weights default to 0. Duplicate text/reading pairs keep the highest weight.
Weights are relative ranking values, not a way to override more accurate matches.
Profile-specific phrases use the same format under `profiles/<profile>/dictionaries/`.

Save the file and select **Reload dictionaries**. No recompilation or installation registry edit is needed.

## Mappings, completions, and emoji

Extension keys need not be valid pinyin, but must contain only lowercase ASCII letters.
Rows use `key output [weight]`, separated by whitespace such as spaces or tabs.
Weights default to 0.

`extensions/exact.tsv` requires the entire key:

```text
addr "Room 12, Main Street" 100
smile 😊 100
xiaolian 😊 100
```

`extensions/completion.tsv` accepts a continuous prefix or a full match:

```text
google Google 1000
good good 800
smile 😊 100
```

Outputs may contain words, English text, or emoji. Quote outputs containing spaces;
escapes follow JSON string rules. Different keys may share an output.
Blank lines and `#` comments are supported.

Shared and current-profile extensions are combined. Select **Reload dictionaries** after editing.
Extensions apply in Pinyin mode, not the English keyboard, and do not use `:smile:` syntax.

Extensions and pinyin are ranked together: full matches precede prefix completions,
which precede abbreviations. Within a class, existing entries precede generated combinations,
then completion distance matters. Equally matched extensions use their weights.
Final results are deduplicated by output.
Extension commits do not add pinyin learning statistics.

## Profiles

The first profile is named `日常` (“Daily”). A profile is a Pinyin dictionary and learning
environment, not another input method.

- **New profile:** create and switch immediately. Shared dictionaries remain available;
  profile-specific data starts empty. Success clears the name field.
- **Rename:** preserve the profile's contents.
- **Duplicate:** copy dictionaries, extensions, personal words, and learning; subsequent learning is independent.
- **Archive:** remove the profile from the active list and retain its files under
  `archived-profiles/<name>-<timestamp>` in both configuration and data roots.
  At least one active profile must remain.

Names allow Chinese characters and spaces, but not empty names, path separators,
control characters, `.`, or `..`. Existing directories are not overwritten.
Large copies can delay this engine; configuration exchanges have a 10-second deadline.

Switching profiles preserves typed pinyin, regenerates candidates, clears short context,
and saves the selection for the next startup. Selecting the current profile is a no-op.
Profiles do not switch automatically by application.
Manually added profile directories require a restart; **Reload dictionaries** updates loaded profiles only.
To restore an archive, exit the engine and move the matching configuration and data directories
back to their respective `profiles/<profile>/` locations.

## Personal vocabulary and learning

Only candidate commits learn. Browsing, paging, canceling, and Enter committing raw input do not.
Learning is profile-specific and combines repeated selections, recency, and short context
from the same input session. One selection does not permanently pin a word.
Focus loss, cancellation, and profile changes do not carry old session context forward.
The engine does not read the application's full text to infer context.

In Settings, select Chinese Pinyin and open **Personal vocabulary**.
Search by text or reading, browse pages, and enter a word and its full reading to add it.
The Pinyin engine must be loaded, but need not be the active input method.

Text and complete reading identify a word. Readings may have tone marks or be untoned;
separate syllables with spaces, for example `shū rù fǎ`.

Available actions: Search, Add, Pin / Unpin, Hide / Show, Forget learning, and Remove entry.

Operations are bound to the profile loaded by the page. Reopen the vocabulary page after
switching profiles; stale operations are rejected rather than applied to another profile.

Changes apply in memory immediately and save in the background.
Normal exit waits for saving; failed writes remain pending and retry.
Settings can display save errors and offer a manual retry.
Forced termination may lose recent changes. Do not edit learning.json or vocabulary.json while running.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Missing phrases or poor sentence results | Add phrase dictionaries, not just the default character table; verify complete readings |
| File edits do not appear | Select **Reload dictionaries**; new profile directories require a restart |
| Extension missing | Check Pinyin mode, lowercase key, full match for exact.tsv, and the active configuration/profile directory |
| Reload fails | Check the reported file and line, tabs, readings, weights, and quotes; the previous dictionary remains active |
| Removed personal word still appears | Removing an entry does not delete base dictionaries; use Hide to suppress it |
| Profile changed while editing | Confirm the active profile and reopen the vocabulary page |
| Saving failed | Check space, permissions, and another instance holding the data directory, then retry saving |

Reload validates characters, shared phrases/extensions, and phrases/extensions for every loaded profile
before replacing anything. Current input and learning remain; the selected word stays selected if still present.
Follow logs with `journalctl --user -u mirix.service -f`.
