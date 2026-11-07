// Fluent messages from the shared locales/ directory (ADR-0007). All user-facing text comes from here.
import { FluentBundle, FluentResource, type FluentVariable } from '@fluent/bundle';
import web from '../../../locales/en/web.ftl?raw';

const bundle = new FluentBundle('en', { useIsolating: false });
const errors = bundle.addResource(new FluentResource(web));
if (errors.length > 0) console.error('locale errors', errors);

export function t(id: string, args?: Record<string, FluentVariable>): string {
  const msg = bundle.getMessage(id);
  if (!msg?.value) return id; // a missing key is visible in the UI and caught by the lint
  return bundle.formatPattern(msg.value, args);
}
