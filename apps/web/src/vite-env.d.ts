/// <reference types="vite/client" />
declare module '*.ftl?raw' {
  const text: string;
  export default text;
}
/** The workspace's version (Cargo.toml), set at build time (vite.config.ts). */
declare const __AGORA_VERSION__: string;
