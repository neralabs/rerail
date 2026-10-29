/**
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

import React from 'react';
import {type Props as DropdownNavbarItemProps} from '@theme/NavbarItem/DropdownNavbarItem';
import type {Types, Props} from '@theme/NavbarItem';

import ComponentTypes from '@theme/NavbarItem/ComponentTypes';

function getComponentType(type: Types, isDropdown: boolean) {
  // Backward compatibility: navbar item with no type set
  // but containing dropdown items should use the type "dropdown"
  if (!type || type === 'default') {
    return isDropdown ? 'dropdown' : 'default';
  }
  return type as NonNullable<Types>;
}

export default function NavbarItem({type, ...props}: Props): JSX.Element {
  const componentType = getComponentType(
    type,
    (props as DropdownNavbarItemProps).items !== undefined,
  );
  // Every navbar item type is a module-level component from the static registry.
  const NavbarItemComponent = ComponentTypes[componentType];
  if (!NavbarItemComponent) {
    throw new Error(`No NavbarItem component found for type "${componentType}".`);
  }
  return <NavbarItemComponent {...(props as never)} />;
}
