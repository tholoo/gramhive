# Prototype boundaries and decisions

GramHive owns application semantics; grammers owns MTProto, Telegram objects,
authentication primitives, sessions, DCs and updates. This repository continues the
original main history; `v1-legacy` and `legacy` preserve the pre-rewrite tip.

## What was inspected

The legacy `Swarm` maintained separate account tasks, shutdown channels and
account-scoped waiters. Account identity and independent lifecycle remain useful
product ideas. Its router spawned every matching route concurrently through a
`dptree::DependencyMap`, while command metadata, regexes and parameter macros grew
into an implicit injection system. Tests mirrored a large portion of the Telegram
client API. GramHive 2 replaces those mechanisms instead of porting them. There was
no reusable localization/lock-chain implementation in the inspected framework;
these fit the new middleware boundary when concrete use cases arrive.

The adapter was implemented after inspecting the published source of
[grammers-client 0.10.0](https://docs.rs/grammers-client/0.10.0/grammers_client/),
its echo example, callback answer builder, peer references, sender pool and SQLite
session APIs. Its [upstream repository is on Codeberg](https://codeberg.org/Lonami/grammers).
A stable release provides the needed modular APIs, so no git dependency is needed.
The upstream callback builder answers then edits; our plan uses independent futures
for answer and edit. Inline text editing uses grammers' generated Telegram RPC type
because its direct inline-edit convenience method is private.

## Small internal model

An event contains account metadata, small routing projections, a scoped driver, and
one transport raw-object provider. It contains no arbitrary application dependency
map. `RawSource` recognizes upstream types; application dependencies live in `State`.

Handlers remain typed only at registration. Each route stores an erased closure;
the router's type depends solely on state. Middleware similarly uses `Next` and
boxed futures. Handler implementations cover up to six arguments. A small internal
response enum is justified here: it represents operations, explicit composition,
progress streams and failure, and keeps one executor shared by tests and production.
It is not an exhaustive wrapper around the Telegram API.

A driver is bound to an event's account and target, so the core does not need
upstream peer references or a cache of Telegram entities. The production driver
retains an upstream Client and Update; `Raw<Client>` clones the cheap upstream
handle. The fake driver records semantic progress plus executed operations.

`Reply` is a small outgoing body (text, optional local document, buttons), also used
as a progress final result. Editing replaces the button markup; empty buttons clear
it. Rich Telegram features remain reachable through raw grammers rather than a
second growing entity hierarchy.

Tower's Service trait provides interoperability. We deliberately use a small erased
Middleware trait instead of exposing Tower's nested generic layer types. Readiness
is always ready, while admission occurs in the returned future. This distinction is
documented so embedding users can choose their own queue/backpressure policy.

## Intentional syntax differences

- `CommandSpec` is the manually implementable command trait; `Command<T>` is the
  extractor and `Command` in the macro namespace is the derive.
- `Router::with_state(state)` is an associated constructor, avoiding missing-state
  generic machinery. `Router::new()` creates a unit-state router.
- `Hive::new(api_id, api_hash)` explicitly supplies Telegram application credentials.
- `SendMessage` avoids shadowing Rust's `Send` trait in the prelude.
- `Button::callback` returns Result because valid typed data can still exceed 64 bytes.
- Progress finishes with `Reply` (including `File` conversion), rather than arbitrary
  response plans: editing a sequence or callback answer into a status is undefined.

## Before expanding scope

1. Decide the application's rejection/error presentation policy. Routing exposes
   distinct outcomes; the runtime currently logs them.
2. Decide progress cancellation and rate/coalescing policy before using it for
   high-frequency or persistent background jobs. Shutdown currently drains work.
3. Choose whether raw operations need a mockable application service layer in each
   app; raw Telegram calls intentionally bypass the fake driver.
4. Decide callback schema migration expectations. Names survive enum reordering,
   but changing fields or variant names requires a new prefix.
5. Dynamic account control, per-chat ordering, richer file/media responses and
   interactive user authentication should follow actual application requirements.

No live Telegram credentials are used by tests. Successful offline checks establish
routing/execution semantics and API compatibility, not a live-account smoke test.
