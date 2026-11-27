// @vitest-environment jsdom
// ADR-0136: a machine's CPU, memory and disks, from its node's reports to its card and to Home's "Needs you".
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { DiskAlertsBlock, MachineGauges } from '../components/MachineGauges';
import { applyEvent, diskAlerts, diskLevel, emptyView, fromSnapshot, machineFiguresOf, machines } from './view';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const GB = 1024 ** 3;
let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

describe('machine figures', () => {
  it('are read from a report, nothing from an older node, and a report without them keeps the last', () => {
    expect(machineFiguresOf({}, 1)).toBeUndefined();
    const f = machineFiguresOf({ cpuCount: 8, cpuBusyPermille: 4000, memoryTotalBytes: 10, memoryAvailableBytes: 99, disks: [{ mount: '/', totalBytes: 100, freeBytes: 500, holdsData: true }, { mount: '/x', totalBytes: 0 }] }, 7);
    expect(f).toEqual({ cpuCount: 8, cpuBusy: 1000, memTotal: 10, memAvailable: 10, workspaces: 0, atMs: 7, disks: [{ mount: '/', total: 100, free: 100, data: true, workspaces: false }] });
    const v = emptyView();
    applyEvent(v, 1, { producerId: 'n1', body: 'nodeOnline', nodeOnline: { nodeId: 'n1', hostname: 'box' } } as never);
    applyEvent(v, 2, { producerId: 'n1', atMs: 50, body: 'nodeResources', nodeResources: { nodeId: 'n1', cpuCount: 4, memoryTotalBytes: 8 * GB, memoryAvailableBytes: 4 * GB, disks: [{ mount: '/', totalBytes: 80 * GB, freeBytes: 2 * GB }] } } as never);
    applyEvent(v, 3, { producerId: 'n1', body: 'nodeResources', nodeResources: { nodeId: 'n1', agentsRunning: 2 } } as never);
    const m = machines(v)[0];
    expect(m?.machine?.cpuCount).toBe(4);
    expect(m?.agentsRunning).toBe(2);
    const s = fromSnapshot({ nodes: [{ nodeId: 'n2', hostname: 'b', online: true, cpuCount: 2, resourcesAtMs: 9, disks: [{ mount: 'C:\\\\', totalBytes: 100 * GB, freeBytes: 50 * GB }] }] } as never);
    expect(machines(s)[0]?.machine).toMatchObject({ cpuCount: 2, atMs: 9, disks: [{ mount: 'C:\\\\' }] });
  });

  it('call a disk low under 10 % or 5 GB free and critical under 3 GB, only on online machines, most urgent first', () => {
    expect(diskLevel({ total: 500 * GB, free: 100 * GB })).toBe('ok');
    expect(diskLevel({ total: 500 * GB, free: 40 * GB })).toBe('low');
    expect(diskLevel({ total: 40 * GB, free: 4.5 * GB })).toBe('low');
    expect(diskLevel({ total: 40 * GB, free: 2 * GB })).toBe('critical');
    const disk = (mount: string, free: number) => ({ mount, total: 100 * GB, free, data: false, workspaces: false });
    const machine = (id: string, online: boolean, disks: ReturnType<typeof disk>[]) => ({ id, name: id, os: 'linux', online, agentsRunning: 0, lastSeenMs: 0, machine: { cpuCount: 1, cpuBusy: 0, memTotal: 1, memAvailable: 1, workspaces: 0, atMs: 1, disks } });
    const alerts = diskAlerts([machine('a', true, [disk('/', 8 * GB), disk('/data', 50 * GB)]), machine('b', true, [disk('/', 1 * GB)]), machine('c', false, [disk('/', 0)])]);
    expect(alerts.map((a) => `${a.machine}${a.mount}:${a.level}`)).toEqual(['b/:critical', 'a/:low']);
  });

  it('show CPU, memory and each disk in words, a low one said so, and the alerts lead Home', () => {
    const figures = { cpuCount: 16, cpuBusy: 372, memTotal: 32 * GB, memAvailable: 16 * GB, workspaces: 12 * GB, atMs: Date.now() - 60_000,
      disks: [{ mount: '/', total: 500 * GB, free: 40 * GB, data: true, workspaces: true }] };
    act(() => root.render(<MachineGauges figures={figures} online={false} now={Date.now()} />));
    const text = host.textContent ?? '';
    expect(text).toContain('37 % · 16 cores');
    expect(text).toContain('16 GB of 32 GB');
    expect(text).toContain('workspaces 12 GB');
    expect(text).toContain('Low: 40 GB free of 500 GB');
    expect(text).toContain('Measured');
    expect(host.querySelector('[data-disk-level="low"] .machine-meter__bar--low')).not.toBeNull();
    act(() => root.render(<MachineGauges online now={Date.now()} />));
    expect(host.textContent).toContain('does not report');
    act(() => root.render(<DiskAlertsBlock alerts={[{ machine: 'b', machineName: 'build', mount: '/', free: 2 * GB, total: 80 * GB, level: 'critical' }]} />));
    expect(host.textContent).toContain('A disk is running low');
    expect(host.querySelector('a[href="#/machines/b"]')).not.toBeNull();
    act(() => root.render(<DiskAlertsBlock alerts={[]} />));
    expect(host.querySelector('[data-disk-alerts]')).toBeNull();
  });
});
