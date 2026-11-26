import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './design/app.css';
import { initTheme } from './themes/store';
import { installSafeKey } from './plugins/safe';
import { pluginStore } from './plugins/store';

// The safe-mode key is heard before anything else on the page (ADR-0114); then the theme, then the plugins this browser loaded (none in
// safe mode).
installSafeKey();
initTheme();
void pluginStore.start();

const root = document.getElementById('root');
if (root) createRoot(root).render(<StrictMode><App /></StrictMode>);
