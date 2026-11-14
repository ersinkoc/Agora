import { expect, it } from 'vitest';
import { ASK, chosenIds, confirmedIds, usableOptions, confirmSkip, initialLaunch, keepAsking, labelOf, pickPermission, toggleHarmless, type LaunchOptionChoice } from './launchChoice';

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
  expect(chosenIds(yes, options)).toEqual(['skip-permissions']);
  expect(confirmedIds(yes, options)).toEqual(['skip-permissions']);
  expect(confirmedIds(asked, options)).toEqual([]);
  expect(chosenIds(pickPermission(yes, ASK), options)).toEqual([]);
});

it('sends harmless options in the kind\'s order and never an id the kind does not offer', () => {
  let s = toggleHarmless(initialLaunch(), 'resume-last', true);
  s = confirmSkip(pickPermission(s, 'skip-permissions'));
  expect(chosenIds(s, options)).toEqual(['skip-permissions', 'resume-last']);
  expect(chosenIds({ permission: 'other', harmless: ['nope'] }, options)).toEqual([]);
  expect(chosenIds(toggleHarmless(s, 'resume-last', false), options)).toEqual(['skip-permissions']);
});

it('shows the id when this window has no message for the label', () => {
  const tr = (id: string) => (id === 'option-skip-permissions' ? "Don't ask" : id);
  expect(labelOf(options[0]!, tr)).toBe("Don't ask");
  expect(labelOf(options[1]!, tr)).toBe('resume-last');
});

it('sends a relabelled option unconfirmed (the machine refuses it) and never offers an id with a bang or a comma', () => {
  // A hub that turned `skipsPermissions` off: the window shows a checkbox and asks nothing, so nothing is confirmed.
  const hidden: LaunchOptionChoice[] = [{ id: 'skip-permissions', label: 'option-resume-last', skipsPermissions: false }];
  const on = toggleHarmless(initialLaunch(), 'skip-permissions', true);
  expect(chosenIds(on, hidden)).toEqual(['skip-permissions']);
  expect(confirmedIds(on, hidden)).toEqual([]);
  // A hub that advertised `skip-permissions!` or `a,b` as harmless: dropped before it is offered or signed.
  const abusive: LaunchOptionChoice[] = [
    { id: 'skip-permissions!', label: 'option-resume-last', skipsPermissions: false },
    { id: 'a,b', label: 'option-resume-last', skipsPermissions: false },
  ];
  expect(usableOptions(abusive)).toEqual([]);
  let s = toggleHarmless(initialLaunch(), 'skip-permissions!', true);
  s = toggleHarmless(s, 'a,b', true);
  expect(chosenIds(s, abusive)).toEqual([]);
  expect(confirmedIds(confirmSkip(pickPermission(s, 'skip-permissions!')), abusive)).toEqual([]);
});
