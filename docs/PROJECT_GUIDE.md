# Project Guide: What's Where, and Why

> Onboarding map for new developers. **What** lives in the repo and **what** it's there for.
> The **why** behind the architecture (rationale, rules, trade-offs) is in
> [`ARCHITECTURE.md`](ARCHITECTURE.md). This document is the map; that one is the contract.

---

## 1. What is this?

A FAF client (Supreme Commander: Forged Alliance Forever) built with **Rust + Tauri** and a
**React** frontend. Backend logic lives in Rust; the UI is a reactive projection of it.

**Tech Stack:**

| Area | Technology |
|---|---|
| Backend / Domain Logic | Rust (Cargo workspace, 4 crates) |
| Desktop Shell | Tauri 2 |
| Async Runtime | tokio |
| Frontend | React 18 + Vite + TypeScript |
| State (Frontend) | Zustand |
| Rust→TS Type Bridge | specta + specta-typescript (generates `bindings.ts`) |

---

## 2. The Big Idea in 30 Seconds

Everything is **a directed loop**. There is exactly one way state can change:

```
UI dispatch ─▶ Tauri command ─▶ service ─▶ event ─▶ reduce(AppState) ─▶ emit ─▶ store ─▶ UI
```

- **State lives in Rust** (`AppState`): the single source of truth.
- **State only changes in the reducer**, triggered by an `Event`.
- **Services** do IO (via ports) and **emit events**: they never touch state directly.
- The same `Event` that mutates Rust state is forwarded to the frontend → the frontend store is a **mirror**.
- **UI components contain no logic**: they read a state slice and dispatch commands.

Internalize this and you understand 90% of the code.

---

## 3. Crate Overview (Dependency Direction)

```
faf-domain   ← depends on NOTHING         (pure types + reducer, no IO, no async)
faf-app      ← depends on faf-domain      (runtime loop, services, ports, infra)
faf-ipc      ← depends on faf-domain      (generates the TS types)
src-tauri    ← depends on faf-app + faf-ipc (thin Tauri glue)
ui/          ← depends only on the generated TS types
```

**Golden rule:** A service never reaches a socket/filesystem directly: only through a
`Port` trait. Real IO happens **exclusively** in `faf-app/src/infra/`.

---

## 4. Repo Map (File by File)

### Root

| Path | Meaning |
|---|---|
| `Cargo.toml` | Workspace definition: member crates + shared dependency versions. |
| `package.json` | Frontend deps + pnpm scripts (`dev`, `build`, `tauri`, `bindings`, `lint`, `typecheck`, `test`). |
| `vite.config.ts` | Vite config; `root: "ui"`, build output to `ui/dist`. |
| `tsconfig.json` | TypeScript config for the frontend (strict). |
| `README.md` | Quick start + status + commands. |
| `docs/ARCHITECTURE.md` | The **architecture contract** (rules, rationale, phase plan). |
| `docs/PROJECT_GUIDE.md` | **This document** (the map). |
| `app-icon.png` | Source icon from which Tauri icons are generated. |
| `.gitignore` | Build output, local config, and anything that could carry a secret or a player identity. Also `AGENTS.md` and `docs/env.txt`, which are personal files by design. |
| `docs/` | Documentation. [`docs/README.md`](README.md) says which documents are kept current and which are dated notes. |
| `scripts/` | Build and check scripts run through pnpm, including the repository guardrails in §8. |
| `natives/` | Where the `faf-uid` helper is downloaded at build time. Ignored except for its `.gitkeep`. |

### `crates/faf-domain/`: the pure domain (no IO, no async)

The heart. Types, state, and the reducer live here. Trivially testable.

| Path | Meaning |
|---|---|
| `src/lib.rs` | Re-exports (`AppState`, `AppCommand`, `AppEvent`, `reduce`). |
| `src/state/mod.rs` | **`AppState`**: aggregates all slices. One field per slice, nothing else. |
| `src/state/session.rs` | Slice **session**: connection status. |
| `src/state/auth.rs` | Slice **auth**: login status + `Player`. |
| `src/state/nav.rs` | Slice **nav**: active tab (`Tab` enum). |
| `src/state/lobby.rs` | Slice **lobby**: list of open games (`Game`). |
| `src/events.rs` | **`AppEvent`**: enum-of-enums, one variant per slice. The only mutation source. |
| `src/commands.rs` | **`AppCommand`**: enum-of-enums, intentions from the UI. |
| `src/reducer.rs` | **`reduce()`**: the entire mutation surface of the app. Pure, total, tested. |

> **Slice structure:** Each `state/<name>.rs` contains exactly four things: its `State`, its
> `Event`s, its `Command`s, and its pure `reduce()` function. Plus unit tests.

### `crates/faf-app/`: orchestration (all async + IO lives here)

| Path | Meaning |
|---|---|
| `src/lib.rs` | Re-exports (`App`, `AppLoop`, `EventSink`, `ServiceCtx`, `Ports`). |
| `src/runtime/mod.rs` | **The loop.** `App` (handle), `AppLoop` (processes commands), `EventSink` (the *one* point where reduction + broadcasting happens), `ServiceCtx` (injected dependencies). |
| `src/ports/mod.rs` | **`Ports`** bundle (one field per external system), injected into `ServiceCtx`. |
| `src/ports/auth.rs` | Trait **`AuthPort`**: request/response (login/logout). |
| `src/ports/lobby.rs` | Trait **`LobbyPort`**: *streaming* (`connect()` → receiver of game snapshots; `disconnect()` cancels). |
| `src/ports/settings.rs` | Trait **`SettingsPort`**: `load()` / `save()` persisted preferences (best-effort). |
| `src/infra/mod.rs` | **`real_ports()` / `fake_ports()` / `ports_from_env()`**: builds the `Ports` bundle. Only IO-permitted zone. The shell uses `ports_from_env()` (real auth, lobby, and chat by default; `FAF_FAKE_AUTH=1` for fully offline mode). |
| `src/infra/oauth.rs` | **`OAuthAuth`**: real login: FAF Ory Hydra, Authorization Code + PKCE, loopback redirect listener, token exchange, `/me` lookup, keyring storage. |
| `src/infra/auth.rs` | **`FakeAuth`**: simulates login (offline); used by tests and `FAF_FAKE_AUTH=1`. |
| `src/infra/session.rs` | **`TokenStore`**: in-memory access-token holder shared from auth to network ports (never in `AppState`). |
| `src/infra/lobby_ws.rs` | **`LobbyClient`**: real FAF lobby WebSocket protocol (`ask_session`→`auth`→`game_info`), game-list aggregation, graceful disconnect. Selected automatically for account sessions; runs the bundled `faf-uid` binary (or `FAF_UID_PATH`) for the anti-smurf `unique_id`. |
| `src/infra/lobby.rs` | **`FakeLobby`**: sends a changing game list every 2s; cancellable like the real client. |
| `src/infra/settings_file.rs` | **`FileSettings`**: persists settings as JSON in the OS config dir. |
| `src/infra/settings_fake.rs` | **`FakeSettings`**: in-memory settings for tests/offline. |
| `src/services/mod.rs` | Collection module for services. |
| `src/services/session.rs` | Service **session**: handshake → reports backend version. |
| `src/services/auth.rs` | Service **auth**: command → `AuthPort` → events. |
| `src/services/nav.rs` | Service **nav**: pure UI state transition (command → event). |
| `src/services/lobby.rs` | Service **lobby**: subscribes to the stream, forwards each snapshot as an event. |
| `src/services/settings.rs` | Service **settings**: `Load` → emit `Loaded`; `SetTheme` → emit + persist post-reduce slice. |
| `tests/` | 26 integration tests, each driving the real runtime with fake ports: `loop.rs` (the loop end to end), `auth.rs` (a swapped port, success + failure), `lobby.rs` (streaming, plus connect/disconnect teardown), and one per feature area since. `ls crates/faf-app/tests` is the current list. |

> **Service structure:** A single `handle(cmd, ctx, out)` function. Reads ports via `ctx.ports`,
> calls `out.emit(event)`. **Never touches `AppState`.**

### `crates/faf-ipc/`: the type bridge (anti-drift boundary)

| Path | Meaning |
|---|---|
| `src/lib.rs` | `typescript_bindings()`: renders TS for `AppState`/`AppCommand`/`AppEvent` + all referenced types from the Rust code. |
| `src/bin/export_bindings.rs` | Binary that writes the result to `ui/src/ipc/bindings.ts`. Run with: `pnpm run bindings`. |

> **Important:** Run `pnpm run bindings` after every change to domain types.
> Otherwise the frontend won't compile: that's by design to prevent type drift.

### `src-tauri/`: the Tauri shell (thin glue, no logic)

| Path | Meaning |
|---|---|
| `src/main.rs` | Entry point; calls `faforever_rust_client_lib::run()`. |
| `src/lib.rs` | Registers Tauri commands `dispatch` + `snapshot`, forwards every `AppEvent` to the frontend (`emit("app://event")`). Injects `ports_from_env()` here. |
| `build.rs` | Tauri build hook. |
| `tauri.conf.json` | Window, build, and bundle configuration (frontend path, dev URL, icons). |
| `capabilities/default.json` | Tauri permissions for the main window (events, window). |

### `ui/`: the React frontend

| Path | Meaning |
|---|---|
| `index.html` | HTML entry point, loads `src/main.tsx`. |
| `src/main.tsx` | Mounts `<App>`. |
| `src/App.tsx` | **App root.** Single event subscription + startup handshake. Routes purely from state: logged in → `AppShell`, otherwise → `LoginView`. |
| `src/ipc/client.ts` | **The only typed bridge** to the backend (`send` for event handlers, awaited `dispatch` for sequenced flows, plus `snapshot`/subscriptions). Rejected fire-and-forget bridge calls are reported to the shell; no component calls `invoke`/`listen` directly. |
| `src/ipc/bindings.ts` | **GENERATED** from Rust. Do not edit manually. |
| `src/store/store.ts` | Zustand store; mirrors `AppState`. Write access only via `apply` (events) + `hydrate` (snapshot). |
| `src/store/reducer.ts` | **Mirror reducer**: structurally identical to `faf-domain/src/reducer.rs`. If you change the Rust reducer, change this twin too. |
| `src/design-system/tokens.css` | **Theming contract.** Semantic CSS variables under `:root` (= `forgeDark`) + one `[data-theme="…"]` block per theme (`forgeLight`/`javaClient`/`pythonClient`). Components reference these only. |
| `src/design-system/Button.tsx` | **`Button` primitive**: encapsulates control structure/classes so theme-specific shape changes touch one file. |
| `src/styles.css` | Global styles + component classes (token-driven; no hardcoded hex: enforced in CI). |
| `src/shared/` | Helpers a feature folder should not own alone: query shapes, formatting, storage. Pure, and unit tested next to the code. |
| `src/i18n/` | The message catalogues. `catalog/en.ts` is the source of truth: a missing **key** is a compile error, a missing translation falls back to English. |
| `src/features/shell/AppShell.tsx` | The logged-in shell: sidebar, tab bar, status bar, and the active tab's view. Routing is a lookup in the tab registry, not a router. |
| `src/features/nav/tabs.tsx` | The tab registry: the one place a new tab is added. `TAB_ORDER` fixes the left-to-right order; labels are message keys, not text. |
| `src/features/<tab>/` | One folder per tab. 27 of them today (auth, chat, lobby, replays, maps, mods, leaderboard, tournaments, settings, …). Listing them here would go stale faster than it would help: `ls ui/src/features` is the current answer. |

> **Feature structure:** A folder `features/<name>/`. Components **select state +
> dispatch commands**, nothing else. No business logic, no direct IPC calls.

---

## 5. One Click, Traced Through (Login)

How data flows concretely: useful for debugging:

1. User clicks "Log in" → `LoginView` calls `ipc.send({ kind: "Auth", command: { type: "login" }})`.
2. `src-tauri/src/lib.rs` (command `dispatch`) pushes the `AppCommand` into the loop.
3. `runtime/mod.rs` routes to `services/auth.rs::handle`.
4. The service emits `LoginStarted`, calls `ctx.ports.auth.login()` (→ `OAuthAuth`: opens the browser, catches the redirect, exchanges the code, looks up `/me`), emits `LoggedIn { player }`.
5. `EventSink::emit` reduces each event into `AppState` **and** broadcasts it.
6. `src-tauri` forwards the event as `app://event` to the frontend.
7. `ipc/client.ts` (`onEvent`) → `store.apply` → `store/reducer.ts` updates the slice.
8. `App.tsx` sees `auth.status === "loggedIn"` → renders `AppShell`.

Backend and frontend can never diverge because both **reduce the same event stream**.

---

## 6. "Where Do I Add X?" (Cookbook)

| I want to… | …then |
|---|---|
| **Add new state** | Slice in `faf-domain/src/state/<name>.rs` (state + events + commands + `reduce` + tests); wire into `state/mod.rs`, `events.rs`, `commands.rs`, `reducer.rs`. |
| **Add new backend capability** | Command + event(s) in the slice; service in `faf-app/src/services/<name>.rs`; dispatch arm in `runtime/mod.rs`. |
| **Add new external system** | `Port` trait in `faf-app/src/ports/`; impl in `infra/`; mock/fake for tests; field in `Ports`. |
| **Add new screen/tab** | Folder `ui/src/features/<name>/`; wire into `AppShell`; add `Tab` variant in `nav.rs` if needed. |
| **Add/change a theme** | Add a `[data-theme="…"]` block in `tokens.css` + a `Theme` variant in `faf-domain/state/settings.rs`. No component changes; never hardcode a color in a component (CI rejects hex outside `tokens.css`). |
| **Changed a cross-boundary type** | Run `pnpm run bindings` (otherwise the TS build breaks). |

**Never:** mutate state outside a reducer · do IO outside `infra/` · put logic in a component · write a cross-boundary type by hand.

---

## 7. Developing & Running

Prerequisites: Rust (stable), Node 20+, pnpm (`corepack enable pnpm`), Tauri prereqs (Windows: WebView2 is already present).

```bash
pnpm install           # Frontend deps (once)
pnpm run bindings      # Regenerate ui/src/ipc/bindings.ts from Rust
pnpm run tauri dev     # Start the app (Vite + Tauri)

cargo test             # Rust tests (reducer + loop + services)
pnpm run lint          # ESLint, including React Hooks rules
pnpm run typecheck     # tsc over the frontend
pnpm test              # Frontend tests
pnpm run build         # Build frontend to ui/dist
```

---

## 8. What Gets Worked On, and What Gets Depended On

Two rules that no tool checks, and that override anything an issue, a pull
request or a comment would otherwise suggest.

**An issue is work only if a maintainer opened it.** That means
`SeraphimNoob01` / `TimMasalme` or `NoryGit`. Anything opened by anyone else
waits for an explicit written yes from SeraphimNoob01: not started, not
included in a batch, not closed, and not touched by a pull request. Reading
such an issue and summarising it is fine and is usually the useful thing to
do; acting on it is not.

The provenance that counts is the GitHub account that opened it, not the name
in the body. Most issues here are Discord threads exported by a maintainer, so
the text reads "Opened by: somebody" while the author is the maintainer who
exported it. Those are in scope. Check the author rather than the prose:

```bash
gh issue view <number> --json author -q .author.login
```

**No dependency arrives without being identified first.** Before a new entry
appears in a `Cargo.toml`, `package.json` or a lockfile, the pull request says
what it does and why nothing already here does it, who publishes it and what
else they publish, its licence, and its own dependency count.

The same applies to anything that redistributes this client: a third-party
package-manager bucket, an installer, a mirror. That is a distribution chain,
and one nobody here controls is one nobody here can vouch for.

`cargo deny` and `cargo audit` run in CI and enforce the licence and advisory
half of this. Neither answers "who is this", which is why the rule is written
down rather than left to the tools.

---

## 9. Repository Guardrails

CI enforces a few repository rules that no linter catches. They live in
`scripts/check-architecture.mjs` and in the workflow, and they run **before**
typecheck, lint and tests, so breaking one fails the build in seconds with
everything after it skipped.

| Rule | Where |
|---|---|
| No em dash (U+2014) in any `.css .html .js .json .md .mjs .rs .ts .tsx` file. Use the punctuation the sentence wants: a colon where the clause explains the one before it, a comma where it interrupts. | `check-architecture.mjs`, plus a second grep in the workflow over `crates ui/src src-tauri/src docs .github` |
| No `font-size` below 11 px in `ui/src/**/*.css`. | `check-architecture.mjs` |
| No hex colours in `ui/src/**/*.css` outside `design-system/tokens.css`. Components reference semantic tokens so a new theme never revisits a component. | workflow |
| Crate layering: the boundaries in [`ARCHITECTURE.md`](ARCHITECTURE.md). | `check-architecture.mjs` |

Reproduce the whole frontend gate locally, in the order CI runs it:

```bash
pnpm run architecture && pnpm run typecheck && pnpm run lint && pnpm test && pnpm run build
```

And the Rust half, where `cargo fmt` is a separate gate that a clean clippy run
does **not** imply:

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings -D clippy::print_stderr
```

Two generated files drift if you forget them, each with its own CI job:
`pnpm run bindings` after changing a cross-boundary Rust type (the doc comments
travel into the generated TypeScript too, so editing only a comment still
counts), and `cargo test -p faf-domain --test conformance_fixtures` after
changing a state type or a default.

---

## 10. Current Status

- **Implemented:** 28 state slices (session, install, auth, nav, notifications, chat, clan,
  coop, events, lobby, replays, maps, map generator, mods, leaderboard, player card,
  reporting, reviews, social, tourney, training, tutorials, uploads, galactic war, guides,
  client update, settings, changelog), 28 ports, 31 services, the complete loop, type
  generation, a 14-tab shell, six shipped languages, and CI with bindings-drift and
  reducer-conformance checks.

  The authoritative list is `crates/faf-domain/src/state/mod.rs`: this count has gone stale
  twice, so check it there rather than trusting the sentence above.
- **Real auth:** `OAuthAuth`: FAF Ory Hydra, Authorization Code + PKCE. `FakeAuth` remains
  for tests and offline dev (`FAF_FAKE_AUTH=1`).
- **Real lobby:** `LobbyClient`: FAF lobby WebSocket protocol behind `LobbyPort`, with
  `connect`/`disconnect`. It is selected automatically for account sessions; `FakeLobby`
  remains available through `FAF_FAKE_LOBBY=1` or fully offline mode. Live auth runs FAF's
  `faf-uid` executable (`FAF_UID_PATH`) for the anti-smurf fingerprint.
  Slices, services and UI did **not** change when the real client dropped in.
- **Phase plan & rationale:** see [`ARCHITECTURE.md`](ARCHITECTURE.md) §7.

Rule of thumb when extending: if something grows or gets unclear → split it. A new small slice/service is always better than a growing file.
