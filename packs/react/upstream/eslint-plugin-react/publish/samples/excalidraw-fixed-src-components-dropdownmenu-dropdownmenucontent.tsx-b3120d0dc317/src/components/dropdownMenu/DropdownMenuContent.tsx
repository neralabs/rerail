import { useOutsideClickHook } from "../../hooks/useOutsideClick";
import { Island } from "../Island";

import { useDevice } from "../App";
import clsx from "clsx";
import Stack from "../Stack";
import React, { useMemo } from "react";
import { DropdownMenuContentPropsContext } from "./common";

const MenuContent = ({
  children,
  onClickOutside,
  className = "",
  onSelect,
  style,
}: {
  children?: React.ReactNode;
  onClickOutside?: () => void;
  className?: string;
  /**
   * Called when any menu item is selected (clicked on).
   */
  onSelect?: (event: Event) => void;
  style?: React.CSSProperties;
}) => {
  const device = useDevice();
  const menuRef = useOutsideClickHook(() => {
    onClickOutside?.();
  });

  const classNames = clsx(`dropdown-menu ${className}`, {
    "dropdown-menu--mobile": device.isMobile,
  }).trim();

  const contextValue = useMemo(() => ({ onSelect }), [onSelect]);

  return (
    <DropdownMenuContentPropsContext.Provider value={contextValue}>
      <div
        ref={menuRef}
        className={classNames}
        style={style}
        data-testid="dropdown-menu"
      >
        {/* the zIndex ensures this menu has higher stacking order,
    see https://github.com/excalidraw/excalidraw/pull/1445 */}
        {device.isMobile ? (
          <Stack.Col className="dropdown-menu-container">{children}</Stack.Col>
        ) : (
          <Island
            className="dropdown-menu-container"
            padding={2}
            style={{ zIndex: 1 }}
          >
            {children}
          </Island>
        )}
      </div>
    </DropdownMenuContentPropsContext.Provider>
  );
};
MenuContent.displayName = "DropdownMenuContent";

export default MenuContent;
