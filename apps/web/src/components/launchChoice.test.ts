import { expect, it } from 'vitest';
import { ASK, chosenIds, confirmSkip, initialLaunch, keepAsking, labelOf, pickPermission, toggleHarmless, type LaunchOptionChoice } from './launchChoice';

const options: LaunchOptionChoice[] = [
  { id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true },
  { id: 'resume-last', label: 'option-resume-last', skipsPermissions: false },
];

it('asks before acting by default and never preselects an option that skips permissions', () => {
  const s = initialLaunch();
  expect(s.permission).toBe(ASK);
  expect(chosenIds(s, options)).toEqual([]);
});

it('chooses "Don\'t ask" only after the confirmation, and Keep asking goes back to asking', () => {
  const asked = pickPermission(initialLaunch(), 'skip-permissions');
  expect(asked.pending).toBe('skip-permissions');
  expect(chosenIds(asked, options)).toEqual([]); // nothing is sent while the question is open
  expect(chosenIds(keepAsking(asked), options)).toEqual([]);
  const yes = confirmSkip(asked);
  expect(chosenIds(yes, options)).toEqual(['skip-permissions!']);
  expect(chosenIds(pickPermission(yes, ASK), options)).toEqual([]);
});

it('sends harmless options in the kind\'s order and never an id the kind does not offer', () => {
  let s = toggleHarmless(initialLaunch(), 'resume-last', true);
  s = confirmSkip(pickPermission(s, 'skip-permissions'));
  expect(chosenIds(s, options)).toEqual(['skip-permissions!', 'resume-last']);
  expect(chosenIds({ permission: 'other', harmless: ['nope'] }, options)).toEqual([]);
  expect(chosenIds(toggleHarmless(s, 'resume-last', false), options)).toEqual(['skip-permissions!']);
});

it('shows the id when this window has no message for the label', () => {
  const tr = (id: string) => (id === 'option-skip-permissions' ? "Don't ask" : id);
  expect(labelOf(options[0]!, tr)).toBe("Don't ask");
  expect(labelOf(options[1]!, tr)).toBe('resume-last');
});

it('sends a relabelled option without the confirmation mark, which the machine refuses', () => {
  // A hub that turned `skipsPermissions` off: the window shows a checkbox and asks nothing, so the id goes without `!`.
  const hidden: LaunchOptionChoice[] = [{ id: 'skip-permissions', label: 'option-resume-last', skipsPermissions: false }];
  expect(chosenIds(toggleHarmless(initialLaunch(), 'skip-permissions', true), hidden)).toEqual(['skip-permissions']);
});
