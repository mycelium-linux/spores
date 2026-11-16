# JARVIS inbox Spore

Deterministic triage for incoming messages. Raw messages go in, curated
notifications and a decision record per message come out. It never sends a
reply.

## Where messages come from

A host adapter writes `inbox.raw:<wearer>` records (`inbox.message.v1`):

```json
{"id":"m1","app":"sms","sender":"Mom","thread":"mom",
 "title":"Mom","body":"Call me when you can?","posted_at_ms":1234}
```

The phone-side source (SMS via the modem on a Linux userspace device) is a host
concern and lands later; `seed` writes the same shape so the policy is testable
today.

## Rules, in order

1. **focus** — if the tray has a focus profile, a message surfaces only when
   its app, category, or thread is allowed, or it is a breakthrough. Otherwise
   it is `hold` and, when the profile says `rollup-held`, one digest item is
   published for the batch.
2. **vip** — `--vip` senders are `urgent` and break focus.
3. **breakthrough** — `--breakthrough` words are `urgent` and break focus.
4. **question** — a body containing `?` is `active`.
5. **sensitive** — `--sensitive` words mark the item `sensitive` (the tray hides
   the body) and are treated as urgent-adjacent only for display, never sent.
6. **thread window** — a second message in the same thread within `--window-ms`
   updates the existing item (`"Mom (2 new)"`) instead of posting twice.

Every decision is written to `inbox:<wearer>` (`inbox.decision.v1`) with the
message id, the rule that fired, the action (`publish`/`update`/`hold`), the
notify id, and `source_spore`.

## Commands

```text
jarvis-inbox seed  [--wearer W] [--now MS]
jarvis-inbox once  [--wearer W] [--vip Mom] [--breakthrough "urgent"] \
                   [--sensitive "code,otp"] [--window-ms N] [--now MS] [--explain]
```

`once` processes every raw message that has no decision yet, so it is safe to
run on a schedule. `--explain` prints one line per decision.
