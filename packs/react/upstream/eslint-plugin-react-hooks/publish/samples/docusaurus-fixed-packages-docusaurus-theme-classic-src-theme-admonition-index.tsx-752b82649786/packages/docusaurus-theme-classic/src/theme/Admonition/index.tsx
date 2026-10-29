/**
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

import React, {type ComponentType} from 'react';
import {processAdmonitionProps} from '@docusaurus/theme-common';
import type {Props} from '@theme/Admonition';
import AdmonitionTypes from '@theme/Admonition/Types';

function getAdmonitionType(type: string): string {
  if (AdmonitionTypes[type]) {
    return type;
  }
  console.warn(
    `No admonition component found for admonition type "${type}". Using Info as fallback.`,
  );
  return 'info';
}

export default function Admonition(unprocessedProps: Props): JSX.Element {
  const props = processAdmonitionProps(unprocessedProps);
  // Every admonition type is a module-level component from the static registry.
  const AdmonitionTypeComponent: ComponentType<Props> = AdmonitionTypes[getAdmonitionType(props.type)]!;
  return <AdmonitionTypeComponent {...props} />;
}
