// The plugin author's types (starter/agora-plugin.d.ts, which `agora plugin new` puts in every plugin folder) say exactly what the
// window's bridge does: each type there is checked here against the window's own definition, both ways, so a change on either side
// that the other does not follow fails `npm run typecheck` (tsc -p plugins). Never run; only compiled. The loader's `agora` object is
// held to `AgoraPlugin.Api` by name in src/plugins/kit.test.ts (the loader is plain JavaScript).
import type { Capability } from '../src/plugins/capabilities';
import type { Answers } from '../src/plugins/projection';
import type { Inbound, Outbound, ReadError } from '../src/plugins/protocol';

/** The same type, read-only lists and fields aside (the frame gets a structured copy, which is neither). */
type Plain<T> = T extends Uint8Array
  ? T
  : T extends readonly (infer U)[]
    ? Plain<U>[]
    : T extends object
      ? { -readonly [K in keyof T]: Plain<T[K]> }
      : T;
/** Exactly the same type: not just assignable either way, which would let an optional field be missing on one side. */
type Same<A, B> = (<T>() => T extends Plain<A> ? 1 : 2) extends <T>() => T extends Plain<B> ? 1 : 2 ? true : false;
const same = <A, B>(ok: Same<A, B>): Same<A, B> => ok;

same<AgoraPlugin.Capability, Capability>(true);
same<AgoraPlugin.ReadError, ReadError>(true);
// Each capability's answer, one by one (so a failure names the capability).
same<AgoraPlugin.Answers['agents.read'], Answers['agents.read']>(true);
same<AgoraPlugin.Answers['usage.read'], Answers['usage.read']>(true);
same<AgoraPlugin.Answers['tasks.read'], Answers['tasks.read']>(true);
same<AgoraPlugin.Answers['changes.read'], Answers['changes.read']>(true);
same<AgoraPlugin.Answers['chat.read'], Answers['chat.read']>(true);
same<AgoraPlugin.Answers['theme.read'], Answers['theme.read']>(true);
same<AgoraPlugin.Answers['pane.context'], Answers['pane.context']>(true);
same<keyof AgoraPlugin.Answers, keyof Answers>(true);
// The messages: the window reads `Inbound` from a frame's `{ agora: 1, … }`, and sends `Outbound`.
same<AgoraPlugin.FrameMessage, Inbound & { agora: 1 }>(true);
same<AgoraPlugin.HostMessage, Outbound>(true);
// The capability names the API takes are the capabilities.
same<Parameters<AgoraPlugin.Api['read']>[0], Capability>(true);
same<AgoraPlugin.Api['granted'][number], Capability>(true);
