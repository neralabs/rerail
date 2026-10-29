/**
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

import React from 'react';
import {createRoot, type Root} from 'react-dom/client';

const POPUP_CONTAINER_ID = 'pwa-popup-container';
let root: Root | undefined;

const getContainer = () => document.getElementById(POPUP_CONTAINER_ID);

const createContainer = () => {
  const container = document.createElement('div');
  container.id = POPUP_CONTAINER_ID;
  document.body.appendChild(container);
  return container;
};

export default async function renderReloadPopup(props: {
  onReload: () => void;
}): Promise<void> {
  const container = getContainer() || createContainer();
  const {default: ReloadPopup} = await import(process.env.PWA_RELOAD_POPUP!);
  if (!root) {
    root = createRoot(container);
  }
  root.render(<ReloadPopup {...props} />);
}
