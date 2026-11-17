# JARVIS brief Spore

One notification, composed from what other Spores already recorded. It reads,
summarises, and says where each line came from.

| Section | Source | Rule |
| --- | --- | --- |
| Vitals | `vitals.view:<wearer>` | freshness is carried through; a non-fresh view is reported as unknown |
| Inbox | `inbox:<wearer>` decisions | counts new / follow-ups / held since the previous brief |
| Weather | newest `weather.current` | global, not per wearer; an absent record is unknown, not clear skies; a record with no published range says `no six-hour range reported`, never `6.1..6.1` |

Nothing is estimated. No averaging two devices, no filling a missing reading
with zero, no guessed temperature, no range where the source reported one
value.

## Commands

```text
jarvis-brief once [--wearer W] [--now MS] [--dry-run]
```

`once` writes `brief:<wearer>` (`jarvis.brief.v1`) with the full sections and
the summary lines, then publishes one tray item unless `--dry-run`. The previous
brief's `generated_at_ms` is the cutoff for "since last brief".
