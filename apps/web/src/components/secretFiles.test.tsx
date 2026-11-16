import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage, t } from '../i18n';
import { readDiff } from '../net/diff';
import { isSecretError, readFailure, type FileItem } from '../net/files';
import { FileBody } from './ChangesView';
import { TreeLevel } from './FilesView';

// ADR-0080 in the window: a secret file is marked and not opened, and its change shows counts without content.
const LIST: FileItem[] = [
  { name: '.env', isDir: false, size: 40, secret: true },
  { name: 'notes.md', isDir: false, size: 12 },
];
const tree = () => renderToStaticMarkup(
  <TreeLevel dir="" depth={0} listings={new Map([['', LIST]])} open={new Set()} list={() => {}} toggle={() => {}} pick={() => {}} status={new Map()} folders={new Set()} />,
);
const row = (html: string, name: string) => (html.match(/<button[^>]*role="treeitem"[^>]*>/g) ?? []).find((b) => b.includes(`data-path="${name}"`)) ?? '';

describe('secret files in the file tree', () => {
  beforeEach(() => setLanguage('en'));

  it('marks a secret entry with a lock, a name and the reason, and keeps it focusable', () => {
    const r = row(tree(), '.env');
    expect(r).toContain('aria-disabled="true"');
    expect(r).toContain('aria-label=".env, kept private"');
    expect(r).toContain('title="Kept private: it may hold a secret"');
    expect(r).not.toMatch(/\sdisabled(=|\s|>)/);
    expect(tree().match(/M4 7\.5h8v6H4z/g)).toHaveLength(1); // one lock icon, on the secret row only
  });

  it('negative control: an ordinary file is a plain, openable row', () => {
    const r = row(tree(), 'notes.md');
    expect(r).not.toContain('aria-disabled');
    expect(r).not.toContain('aria-label');
    expect(r).toContain('title="notes.md"');
  });

  it('says the same in Turkish', () => {
    setLanguage('tr');
    expect(t('files-secret')).toBe('Gizli tutuluyor: bir sır içeriyor olabilir');
    expect(row(tree(), '.env')).toContain('aria-label=".env, gizli tutuluyor"');
  });
});

describe('a stale client that still asks for a secret file', () => {
  it('shows the private-file message for files.secret and the general one for anything else', () => {
    const secret = Object.assign(new Error('files.secret'), { code: 'files.secret' });
    expect(isSecretError(secret)).toBe(true);
    expect(readFailure(secret)).toBe('secret');
    expect(readFailure(Object.assign(new Error('x'), { code: 'files.too_large' }))).toBe('failed');
    expect(readFailure(new Error('files.secret'))).toBe('failed');
    expect(readFailure(undefined)).toBe('failed');
  });
});

const DIFF = `diff --git a/.env b/.env
index 3f2a1b0..9c8d7e6 100644
diff --git a/src/a.ts b/src/a.ts
--- a/src/a.ts
+++ b/src/a.ts
@@ -1,1 +1,1 @@
-old
+new
`;

describe('secret files in the review screen', () => {
  beforeEach(() => setLanguage('en'));
  const body = (path: string) => {
    const file = readDiff({ diff: DIFF, secretFiles: [{ path: '.env', added: 2, removed: 1 }] }).find((f) => f.path === path);
    return renderToStaticMarkup(<FileBody file={file as never} layout="unified" wrap={false} text={undefined} opened={new Set()} canExpand onOpenGap={() => {}}
      showAll={false} onShowAll={() => {}} single={undefined} onLoadSingle={() => {}} onRetryText={() => {}} />);
  };

  it('keeps the node\'s counts on the file and drops any content', () => {
    const [env, ts] = readDiff({ diff: DIFF, secretFiles: [{ path: '.env', added: 2, removed: 1 }] });
    expect(env).toMatchObject({ path: '.env', secret: true, added: 2, removed: 1, lines: [], hunks: [] });
    expect(ts).toMatchObject({ path: 'src/a.ts', added: 1, removed: 1 });
    expect(ts?.secret).toBeUndefined();
  });

  it('shows "changed, content hidden" and no diff body', () => {
    const html = body('.env');
    expect(html).toContain('Changed, content hidden');
    expect(html).toContain('Kept private: it may hold a secret');
    expect(html).not.toContain('font-mono');
  });

  it('negative control: a changed ordinary file still shows its lines', () => {
    const html = body('src/a.ts');
    expect(html).not.toContain('content hidden');
    expect(html).toContain('new');
  });

  it('has the Turkish words', () => {
    setLanguage('tr');
    expect(t('changes-secret')).toBe('Değişti, içerik gizli');
  });
});
