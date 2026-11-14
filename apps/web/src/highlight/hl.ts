// The syntax highlighter (highlight.js core and the languages `net/highlight.ts` names), loaded on demand by `useHighlight` so it
// is a chunk of its own and costs nothing until a file is shown. Its output is escaped text inside <span class="hljs-..."> tags.
import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import c from 'highlight.js/lib/languages/c';
import cpp from 'highlight.js/lib/languages/cpp';
import csharp from 'highlight.js/lib/languages/csharp';
import css from 'highlight.js/lib/languages/css';
import diff from 'highlight.js/lib/languages/diff';
import dockerfile from 'highlight.js/lib/languages/dockerfile';
import go from 'highlight.js/lib/languages/go';
import ini from 'highlight.js/lib/languages/ini';
import java from 'highlight.js/lib/languages/java';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import kotlin from 'highlight.js/lib/languages/kotlin';
import makefile from 'highlight.js/lib/languages/makefile';
import markdown from 'highlight.js/lib/languages/markdown';
import php from 'highlight.js/lib/languages/php';
import powershell from 'highlight.js/lib/languages/powershell';
import protobuf from 'highlight.js/lib/languages/protobuf';
import python from 'highlight.js/lib/languages/python';
import ruby from 'highlight.js/lib/languages/ruby';
import rust from 'highlight.js/lib/languages/rust';
import scss from 'highlight.js/lib/languages/scss';
import sql from 'highlight.js/lib/languages/sql';
import swift from 'highlight.js/lib/languages/swift';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';

const LANGUAGES = { bash, c, cpp, csharp, css, diff, dockerfile, go, ini, java, javascript, json, kotlin, makefile, markdown, php, powershell, protobuf, python, ruby, rust, scss, sql, swift, typescript, xml, yaml };
for (const [name, lang] of Object.entries(LANGUAGES)) hljs.registerLanguage(name, lang);

/** Highlighted HTML of `text` in `language`, or undefined when the language is not loaded or the highlighter gives up. */
export function highlight(text: string, language: string): string | undefined {
  if (!hljs.getLanguage(language)) return undefined;
  try {
    return hljs.highlight(text, { language, ignoreIllegals: true }).value;
  } catch {
    return undefined;
  }
}
