// "Follow the tail only while the reader is at the bottom": the rule behind the conversation's scrolling. While the reader is at the
// bottom, new entries keep it there; once they scroll up to read, nothing moves under them and a "Jump to latest" pill counts what
// arrived. Pure, so the rule is tested without a page.

/** How close to the end still counts as being at the end (a trackpad or a half-loaded image never lands exactly on it). */
export const NEAR_BOTTOM = 64;

export interface Follow { following: boolean; unseen: number }
export const start: Follow = { following: true, unseen: 0 };

export const atBottom = (m: { scrollTop: number; clientHeight: number; scrollHeight: number }, slack = NEAR_BOTTOM): boolean =>
  m.scrollHeight - m.scrollTop - m.clientHeight <= slack;

/** The reader scrolled: following again exactly when they are back at the bottom. */
export const scrolled = (f: Follow, bottom: boolean): Follow => (bottom ? { following: true, unseen: 0 } : { following: false, unseen: f.unseen });

/** `added` new entries arrived: they are seen at once while following, otherwise they are counted for the pill. */
export const grew = (f: Follow, added: number): Follow => (f.following || added <= 0 ? f : { following: false, unseen: f.unseen + added });

/** The reader sent something or pressed "Jump to latest": back to the tail. */
export const jumped = (): Follow => ({ ...start });

/** After older entries were drawn above the reader, the scroll position that keeps the same line in view. */
export const keepPosition = (scrollTop: number, heightBefore: number, heightAfter: number): number => scrollTop + (heightAfter - heightBefore);
