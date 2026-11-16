# Watch bridge: decisions to the wrist

This is a surface adapter, not a domain. The bus carries **queries**, **replies**, **decisions** and **notices**; the watch is one place they can appear. Nothing in the contract is named after the watch. No firmware fork: stock InfiniTime already exposes both halves of the adapter.

## The domain

A **decision** needs an answer:

```json
{
  "decision_id": "d-01j9...",
  "question": "Deploy jarvis-api 1.4.2?",
  "summary": "Deploy 1.4.2? Yes recommended. Two minutes down.",
  "detail": "main is green, 42 tests pass, one migration is irreversible.",
  "options": ["yes", "no"],
  "recommendation": "yes",
  "consequence": "Two minutes of downtime",
  "person": "ajmwagar",
  "provenance": {"source": "jarvis", "task": "deploy", "devices": ["omi", "ring-left"]},
  "conversation_ref": {"session_id": "s-42", "turn_id": "t-17"},
  "created_at_ms": 1762800000000,
  "expires_at_ms": 1762800090000,
  "state": "pending",
  "resolution": null
}
```

A **reply** is a final answer addressed to a surface; the watch shows it as a
SimpleAlert only when `state` is `final` and `delivery.surface` is `watch` (or
`all`). Partials stay on the voice hot path, not the wrist.

A **notice** does not:

```json
{
  "notice_id": "n-01j9...",
  "title": "Build",
  "body": "main is green",
  "provenance": {"source": "jarvis", "task": "ci"},
  "created_at_ms": 1762800000000,
  "expires_at_ms": 1762803600000
}
```

`summary` is the short form and `detail` the long form. Surfaces choose. The
100-byte limit is a **watch** rule; it does not belong in the domain schema.

### Bus kinds

| Kind | Payload | When |
|---|---|---|
| `jarvis.query.v1` | query | the user asks or commands |
| `jarvis.reply.v1` | reply | JARVIS answers, final or partial |
| `jarvis.notice.v1` | notice | a notice is created |
| `jarvis.decision.v1` | decision | a decision is created |
| `jarvis.delivery.v1` | `{item_id, surface, state, at_ms}` | a surface showed, resolved or expired it |
| `jarvis.decision.resolve.v1` | `{decision_id, answer, by: {surface, actor}, at_ms}` | an answer, from any surface |

`resolve` is the only mutating event. First resolution wins; a later answer for
the same `decision_id` is a no-op. That is what stops a spoken "yes" and a watch
tap from both firing the deploy. `by.surface` records where the answer came
from; it is not part of the identity of the answer.

## Queries and replies

The inbound direction is the same contract, not a special case. A **query** is
what the user asked, from which surface, with which context; a **reply** is
what JARVIS said, delivered to whichever surface(s) should show or speak it.

```json
// jarvis.query.v1
{
  "query_id": "q-01j9...",
  "text": "what's this?",
  "origin": {"surface": "voice", "node": "s22"},
  "person": "ajmwagar",
  "conversation_ref": {"session_id": "s-42"},
  "context_refs": ["d-01j9..."],
  "created_at_ms": 1762800000000
}

// jarvis.reply.v1
{
  "reply_id": "r-01j9...",
  "query_id": "q-01j9...",
  "text": "Deploy 1.4.2. Yes recommended; two minutes of downtime.",
  "state": "final",
  "delivery": {"surface": "voice"},
  "refs": ["d-01j9..."],
  "at_ms": 1762800001200
}
```

`state` is `partial` or `final`; partials let a slow reply update the same
`reply_id` without inventing a new turn. `query_id` is the idempotency key.

### Transport, not just vocabulary

`soil:unibus` is poll-and-cursor, not request/response. Do not put the voice
hot path through a poll hop: Sound should keep its direct STT → agent → TTS
path and publish `jarvis.query.v1` after admission and `jarvis.reply.v1` as it
finalises. The bus is the shared record and fan-out, not the RPC.

Non-voice origins (Canvas, terminal, phone) can use the bus as their actual
input path; their latency budget is human, not conversational.

### Ownership

The conversation transcript stays where it already lives: Sound owns the raw
turns, Subconscious owns session routing. Query and reply events are the
cross-surface projection of those turns and carry `conversation_ref` back to
the canonical record. Do not create a second transcript store.

A decision is a gated step *inside* a reply or a notice, not a separate
conversation. The chain is `query_id` → optional `decision_id` → `reply_id`,
and every object keeps the same `conversation_ref`.

The watch can resolve a decision but cannot originate a free-text query; it has
no microphone or keyboard. Do not pretend its Accept/Reject buttons are a query
surface. `jarvis.query.v1` is therefore not an input to this adapter; a question
that needs a wrist answer is a `jarvis.decision.v1` (or an `soil:agents` prompt
normalized into one).

## Surfaces, not the domain

A surface adapter subscribes to `jarvis.notice.v1` and `jarvis.decision.v1` and
maps them to its medium:

| Surface | Notice | Decision | Limits |
|---|---|---|---|
| Watch | ANS SimpleAlert | ANS Call with Accept/Reject/Mute | one live call screen, 100-byte summary, no remote dismiss |
| Canvas | card | card with buttons | many cards, full detail |
| Voice | spoken line | spoken prompt | one prompt at a time |
| Phone | notification | notification with actions | platform limits |

The "one question at a time" rule is a **watch** rule, not a global one. The
domain can hold several live decisions; the voice resolver can refer to any of
them, and must ask which one if two are live.

## Why stock InfiniTime can do the watch half

InfiniTime is an ANS **client** as well as a server:

| Item | Value |
|---|---|
| Service | ANS `0x1811` |
| New Alert | `0x2A46`, write |
| Notification event | `00020001-78fc-48fe-8e23-433b3a1942d0`, notify |
| Categories | SimpleAlert `0x00`, Email `0x01`, News `0x02`, Call `0x03`, MissedCall `0x04`, SMS `0x05`, VoiceMail `0x06`, Schedule `0x07`, HighPriority `0x08`, InstantMessage `0x09` |
| Call responses | Reject `0x00`, Answer `0x01`, Mute `0x02` |

A `Call`-category notification renders the incoming-call screen with
**Accept / Reject / Mute** buttons. Tapping one notifies the event
characteristic with one byte. That is the yes/no channel.

Source: `src/components/ble/AlertNotificationService.{h,cpp}` and
`src/displayapp/screens/Notifications.cpp`.

### Frame layout

New Alert is `[category, count, 0x00, title, 0x00, body, 0x00]`. The text is
`title\0body\0`; InfiniTime starts copying at byte 3 (`headerSize = 3`), one more
than the ANS spec's two-byte header. That third byte is skipped, so send a zero
there. **Verify this on the real watch before trusting it** — it is the one
protocol detail that is not obvious from the spec.

Text is capped at 100 bytes total (`NotificationManager::MessageSize`), title
and body combined. The adapter uses `summary` as the title and `detail` (or
`consequence`) as the body, trimming the body to whatever the title leaves.

### Display kind

Stock InfiniTime skips the third header byte. The adapter uses it as a display
hint, so the same ANS category can render differently on the fork without
changing the reply channel:

| Byte | Meaning | Stock | Fork |
|---|---|---|---|
| `0` | plain | category screen | category screen |
| `1` | approval | Call screen | thumbs up/down |
| `2` | choice | Call screen | reserved for a choice list |

A canonical yes/no (or approve/deny) decision sends `1`; an arbitrary binary
choice (`ship` / `hold`) and a decision with more than two options stay `0`
until the fork can honestly render the options. The response bytes remain
`0x01` accept/up, `0x00` reject/down, `0x02` mute/dismiss.

## Adapter shape

The adapter is the `pinetime` Spore (`dev.fungos.pinetime`); the name is fine
for the adapter, because it is the adapter that is watch-shaped, not the bus.
Grants:

- `soil:unibus` — poll the configured kinds and publish delivery/resolution
  events.
- `soil:ble` — connect, write `0x2A46`, subscribe to the event characteristic.
- `soil:records` — persist the active decision and the dedupe set.
- `soil:time` — monotonic poll timing and receipt stamps.

No wall clock, filesystem, network, or credentials.

Loop:

1. Poll `jarvis.notice.v1`, `jarvis.decision.v1`, `jarvis.reply.v1`, and
   `jarvis.decision.resolve.v1`.
2. Map a notice, or a final reply addressed to the watch, to a SimpleAlert.
3. For a decision, enqueue it. If no call screen is live, send the decision
   notification, publish `jarvis.delivery.v1` with `surface: "watch"` and
   `state: "shown"`, and wait for one response byte.
4. Publish `jarvis.decision.resolve.v1` with `by.surface: "watch"`, publish the
   `resolved` delivery, show a receipt, and advance to the next queued item.
   A response from another surface clears the matching active or queued item;
   this adapter's own resolve frame is ignored so it does not double-count.

## Voice binding

Sound keeps a deictic focus: the most recent decision with a `delivery` in
state `shown`, for the same person, inside its TTL. An underspecified "what's
this?" or "yes" binds to that decision. If two are live, ask which one rather
than guessing.

Watch answers are injected into the conversation as a system turn ("Approved
*Deploy jarvis-api* on the watch at 10:09"), so the next voice turn has the
same referent. Voice answers publish the same `resolve` event with
`by.surface: "voice"`, so every adapter can render the resolved state.

Person scoping matters: the watch belongs to one wearer. A guest saying "yes"
must not resolve the wearer's prompt.

## Clearing the watch

Stock InfiniTime has no remote dismiss: ANS can create notifications, never
remove them. On a verbal resolution the adapter can send a replacement
notification ("✓ Approved — deploy started") which becomes the visible item,
but the original stays in the notification list until dismissed by hand.

If that residual entry proves annoying, the smallest honest firmware change is
a dismiss control point (one characteristic or an ANS extension). That is a
single, well-scoped capability and fits the fork policy in `jarvis-erv`; it is
not a reason to own the firmware.

## Host posture

The CAT S22 will own the BLE link, so the adapter Spore belongs there: run it
where the radio is. `soil:ble` already spans CoreBluetooth and BlueZ, so the
same Spore moves from the Mac dev host to the S22 unchanged. The watch's
single-central limit is about the watch, not the host; the S22 can hold the
watch, both rings and the OMI at the same time.

## Implementation

`spores/pinetime` implements the watch half as `pinetime bridge`. The pure
mapping lives in `spores/pinetime/src/bridge.rs`; the poll/BLE loop lives in
`src/lib.rs`. It imports `soil:ble`, `soil:records`, `soil:time`, and
`soil:unibus`; the bridge polls the four JARVIS kinds, maps notices and final
replies to SimpleAlert and canonical decisions to Call, and publishes
delivery/resolution frames. It keeps one decision active, queues up to eight,
trims to the 100-byte watch budget, filters by person, and persists the active
item plus the content hashes already applied in the `jarvis.state.v1` record in
the `bridge:<wearer>` collection, so a guest restart does not lose the question
on the wrist and a retained bus frame is not replayed.

The receiver loop is bounded: it polls every `--poll-ms` (minimum 50 ms), waits
for a response for at most the remaining `--hold-ms`, and runs until stopped when
`--hold-ms 0`. Expiry uses a wall anchor derived from the first bus frame's
`issued_at_ms` plus monotonic elapsed time, so the Spore never needs a wall
clock grant.

The composition is `soil-dev-host`, which links exactly those four imports. The
simulator's scripted call-response event at tick 10 resolves a fixture decision
end to end, and the resulting `jarvis.delivery.v1` and
`jarvis.decision.resolve.v1` frames are visible in the dev host's output.

## Integration gap

`soil-ble-host` links only `soil:ble` plus WASI, so it cannot run `bridge`; that
mode belongs in the composed host. `soil-dev-host` is the bench composition and
supports `--bus-fixture` for a decision without a live Unibus. The production
host still needs a long-lived device supervisor that owns the records store and
bus across guest restarts.

## Test plan

1. Send a SimpleAlert from a bench script; confirm title/body rendering and
   settle the three-byte header question. **Written to the dev watch; rendering
   confirmation pending.**
2. Send a Call-category decision; confirm the Accept/Reject/Mute screen and read
   the response byte on `00020001`. **Simulator path verified; hardware pending.**
3. Run the Spore against the dev kit: decision in, notification out, resolution
   back. **Verified with `soil-dev-host` + simulator, including notice, final
   reply, and a scripted Accept response.**
4. Test queue, cooldown and timeout with a mock bus and a mock link before the
   sealed watch is involved. **Queue, person filter, dedupe, and expiry are
   implemented; cooldown and hardware timing are still open.**

The `Glance` face already reads `NotificationManager::GetLastNotification()`,
so the same decision also becomes the watch face's hero.
