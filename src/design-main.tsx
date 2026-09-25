// Development-only entry point for the design preview mode. Mounted by
// design.html, never by index.html. Installs the Tauri API mocks from
// @tauri-apps/api/mocks before anything else, then renders the exact
// same chrome components BlueFlame uses in the desktop app, so the
// whole UI can be seen, screenshotted and reviewed in a plain browser
// tab. See docs/DESIGN_PREVIEW.md.
//
// This file is not referenced by index.html and is not part of the
// `pnpm build` rollup entry, so it never ships in the production
// bundle; scripts/check-no-design-in-build.mjs asserts that.
import React from 'react';
import ReactDOM from 'react-dom/client';
import { emit } from '@tauri-apps/api/event';
import App from './App';
import { ContextMenu } from './components/ContextMenu';
import { MenuPopup } from './components/MenuPopup';
import { TrustPopup } from './components/TrustPopup';
import { DesignSwitcher, SCREENS, type ScreenId } from './design/DesignSwitcher';
import {
  installDesignMocks,
  emitApprovalRequest,
  emitClaudeTabBadge,
  emitDownloadProgress,
  removeAutoBootTab,
  type Scenario,
} from './design/mockBackend';
import './App.css';
import './design/design.css';

const qs = new URLSearchParams(window.location.search);
const panelParam = qs.get('panel');
const screenParam = (qs.get('screen') as ScreenId | null) ?? 'dashboard';
const screen = SCREENS.some((s) => s.id === screenParam) ? screenParam : 'dashboard';

const scenarioByScreen: Partial<Record<ScreenId, Scenario>> = {
  'trust-modal': 'trust-modal',
  approval: 'approval',
};
const mockState = installDesignMocks(
  scenarioByScreen[screen] ?? 'default',
  screen === 'mobile' || screen === 'tabswitcher',
);

function selectSidebarView(view: string) {
  const btn = document.querySelector<HTMLButtonElement>(
    `aside[aria-label="primary navigation"] button[aria-label="${view}"]`,
  );
  btn?.click();
}

function openFirstTab() {
  const btn = document.querySelector<HTMLButtonElement>('.tab-strip button[role="tab"]');
  btn?.click();
}

function openMobileTabSwitcher() {
  const btn = document.querySelector<HTMLButtonElement>('.mobile-tabs');
  btn?.click();
}

/** Drives the mounted <App/> into the requested screen by dispatching the
 * same clicks/events a person would, since App owns all of this as
 * internal state rather than reading it from the URL. Runs once, shortly
 * after mount, so the initial data load from the mocked backend has
 * already landed.
 *
 * App.tsx's own auto-boot effect (opening a fresh tab when it sees zero
 * tabs on first render) always fires once before the mocked tab list
 * has loaded, since state starts empty and invoke() cannot resolve
 * synchronously in the same commit. That is true against a real backend
 * too, not a design-preview artifact, but it does mean every screen
 * needs an explicit click back to its intended state rather than
 * assuming the fixture tabs are what is showing after mount. */
function driveToScreen(id: ScreenId) {
  window.setTimeout(() => {
    removeAutoBootTab(mockState);
    switch (id) {
      case 'dashboard':
      case 'mobile':
      case 'trust-modal':
      case 'approval':
        selectSidebarView('dashboard');
        if (id === 'approval') window.setTimeout(emitApprovalRequest, 150);
        break;
      case 'bookmarks':
      case 'downloads':
      case 'metrics':
      case 'settings':
      case 'debug':
        selectSidebarView(id);
        if (id === 'downloads') window.setTimeout(emitDownloadProgress, 150);
        break;
      case 'browsing':
        openFirstTab();
        break;
      case 'tabswitcher':
        window.setTimeout(openMobileTabSwitcher, 150);
        break;
    }
  }, 50);
}

driveToScreen(screen);
// The Claude-driven tab badge is part of the "several tabs" fixture, so
// it shows on every screen that renders the desktop tab strip.
window.setTimeout(emitClaudeTabBadge, 100);

function DesignRoot() {
  return (
    <>
      <App />
      <DesignSwitcher current={screen} />
    </>
  );
}

const root = document.getElementById('root') as HTMLElement;

if (panelParam === 'trust') {
  ReactDOM.createRoot(root).render(<TrustPopup />);
} else if (panelParam === 'menu') {
  ReactDOM.createRoot(root).render(<MenuPopup />);
} else if (panelParam === 'context') {
  window.setTimeout(() => {
    void emit('context-menu:payload', {
      page_url: 'https://github.com/w1ck3ds0d4/BlueFlame',
      link_url: 'https://github.com/w1ck3ds0d4/BlueFlame/blob/main/README.md',
      link_text: 'README.md',
      image_url: null,
      selection_text: null,
      screen_x: 120,
      screen_y: 80,
    });
  }, 50);
  ReactDOM.createRoot(root).render(<ContextMenu />);
} else {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <DesignRoot />
    </React.StrictMode>,
  );
}
