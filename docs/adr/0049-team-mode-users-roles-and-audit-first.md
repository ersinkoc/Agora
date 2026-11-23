# 0049. Team mode: users, roles and an audit trail first, Postgres only when SQLite is not enough
Status: Accepted
Date: 2026-11-10
Amended: 2026-11-23 (Amendment A below: the maintainer's decision on the question in #1049)

## Context
`docs/vision.md` lists a team (shared hub, several users, policies, audit) as a later phase, and the roadmap pairs it with
"Postgres". Today the hub is one owner's: sign-in codes and passkeys (ADR-0015), trusted device keys per node (ADR-0028), signed
grants (ADR-0041), and an event store behind the `EventStore` trait with a SQLite implementation. Nothing says who did what beyond
the device that signed it, every signed-in client sees every project, and `docs/architecture.md` promises an append-only audit log
that does not exist as a view or export. Postgres is a storage choice; what a team actually needs is identities, limits on what each
may see and do, and a record. Building the storage first would cost a heavy dependency and a second backend to keep correct
before any of that exists.

## Decision
1. **Order of work:** (T1) users and roles on the existing SQLite hub; (T2) the audit trail, viewable and exportable; (T3) a Postgres
   `EventStore` only when a measured need appears (many users, hub high availability), behind the trait that exists. Each step ships
   alone and leaves a single-owner hub unchanged: a hub with one user and no roles file behaves as today.
2. **Users and roles:** a user is an id, a display name and the devices they enrolled (ADR-0028 devices gain an owner). Three roles:
   `owner` (everything, manages users), `member` (works in the projects granted to them: start and stop agents, type, approve what
   their devices sign), `viewer` (read only; no commands, no approvals). Roles are held by the hub and shown in the page; they are
   not a security boundary on their own.
3. **The node stays the authority (ADR-0020):** a node trusts device keys, not roles. What a role may not do is enforced twice: the
   hub refuses the routed command, and the node's policy lists which trusted devices (hence users) may sign grants and approvals for
   it. A compromised hub can neither promote a viewer nor forge a member's signature.
4. **Project visibility:** a project is visible to the users the owner adds to it; the hub filters snapshots and events per user
   before they leave it. Nodes are machines, not users; an owner sees all of them, members see the machines that run their projects.
5. **Audit trail:** every routed command, approval decision, grant, sign-in and role change is appended with who (user, device),
   what, where (node, project, agent) and the outcome, in an append-only table that no client command can edit or delete. A page lists
   and filters it, `agora hub audit export` writes it as JSON lines. Terminal bytes and message bodies are not in it, only that they
   happened (ADR-0022: bytes never go through the event store).
6. **Sign-in:** passkeys and one-time codes stay; OIDC is added only for team mode and only as an additional way to prove a user id,
   never a replacement for the device key that signs approvals.
7. **No cloud or multi-tenant mode here.** That is separate and out of scope.

## Consequences
- Needs protocol additions (a user on enrollment and on command audit lines, role errors such as `hub.role_denied`) and schema
  fixtures; each slice carries them and an ADR note if the schema changes.
- The hub's filtering per user is a new place where a bug leaks data; tests must prove a member never receives another project's
  events, through the snapshot and the live stream.
- Postgres is deferred rather than rejected: the trait stays the seam, so nothing built now blocks it.
- A single-owner hub pays nothing: no roles file, no filtering, the audit table is the only addition and is small.

## Alternatives considered
- **Postgres first:** the roadmap's wording, but it adds a server to run and a dependency before the features that would use it.
- **One shared owner account:** what teams do today; no audit of who acted and no way to give someone read-only access.
- **Roles enforced only by the hub:** simpler, but a hub compromise would then grant everything, against ADR-0020.
- **Full ACLs per object:** more flexible than three roles and project membership, and far more to build, explain and get wrong.

## Amendment A (2026-11-23, #1049): a member chooses folders and starts agents in their projects
**Why.** Decision 2 lets a member "work in the projects granted to them", but the hub refused every folder command to members
(`DirectoryList`, `DirectoryInspect`, `AgentLaunch`, ADR-0052/0053), so a member could not choose a folder or start an agent in one even
inside a granted project. The maintainer decided on 2026-11-23 (relayed to the agent fixing #1049): yes, a member with a project grant
may browse that project's folders on its machine and start agents there; viewers stay read-only; nothing outside the granted project
is allowed.

**Decision** (the maintainer's answer above; points 1 to 6 are how #1049 implements it, for the maintainer's review with that PR).
1. The hub lets a member's `DirectoryList`, `DirectoryInspect` and `AgentLaunch` through only when the machine the request names is
   the one the grant names and the folder is one of that project's folders on it or below one. A project's folders are the ones that
   machine reported for it (its checkouts, and each workspace's repository root, or its folder when it is no repository), as
   `agora hub user projects` lists them. The nearest reported folder that holds the path decides, as a node places a folder for a
   device limited to some projects: a folder of another project inside a granted one is that other project's, and a folder reported
   for two projects is neither's.
2. The path must be absolute and plain in that machine's spelling: no `.` or `..` part, `/` only as a separator on a Unix machine,
   and on Windows no part ending in a dot or a space and no `:` but the drive's. An empty path (the home folder) is refused.
3. The machine follows links. A listing or a folder's details is therefore checked again in the machine's answer, which names the
   folder it really opened: when that folder is not inside the granted project (a link inside the project that leads out), the member
   gets `auth.forbidden` instead of the answer; a listing whose way up leads out of the project keeps no way up. A start needs no
   second check: a node starts only in the exact real folder named and refuses a path through a link, and the start must name the
   project the hub found for the folder, which the node checks against the folder's own identity.
4. As a node refuses them to a limited device, a member may not make a new project identity (`create_project_id`) or accept a
   changed one (`accept_identity_change`).
5. A refusal stays `auth.forbidden` with `action` set to the refused command's schema name, so clients word it for the person's role
   (#1044). A viewer's folder commands are refused as before, granted project or not.
6. This is the hub's check. The node stays the authority (decision 3): a member's device that the node's policy limits to some
   projects is checked there again by the node's own records.
