import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITerminalOptions, type ITheme } from "@xterm/xterm";
import { useCallback, useEffect, useRef } from "react";

/** Resolve CSS through the browser: xterm cannot consume var() or color-mix(). */
const terminalTheme = (host: HTMLElement): ITheme => {
  const probe = document.createElement("span");
  probe.hidden = true;
  host.append(probe);
  const color = (token: string) => {
    probe.style.color = `var(--${token})`;
    return getComputedStyle(probe).color;
  };
  const theme: ITheme = {
    background: color("background"),
    foreground: color("foreground"),
    cursor: color("brand"),
    selectionBackground: color("muted"),
    selectionForeground: color("foreground"),
    cursorAccent: color("background"),
    black: color("background"),
    brightBlack: color("muted-foreground"),
    red: color("danger-foreground"),
    brightRed: color("danger-foreground"),
    green: color("success-foreground"),
    brightGreen: color("success-foreground"),
    yellow: color("warning-foreground"),
    brightYellow: color("warning-foreground"),
    blue: color("info-foreground"),
    brightBlue: color("info-foreground"),
    magenta: color("brand-foreground"),
    brightMagenta: color("brand-foreground"),
    cyan: color("info-foreground"),
    brightCyan: color("info-foreground"),
    white: color("foreground"),
    brightWhite: color("foreground"),
  };
  probe.remove();
  return theme;
};

type TerminalCallbacks = {
  onInput: (data: string) => void;
  onResize: (cols: number, rows: number) => void;
};

export const usePlaygroundTerminal = ({ onInput, onResize }: TerminalCallbacks) => {
  const terminalHostRef = useRef<HTMLElement>(null);
  const terminalRef = useRef<Terminal | null>(null);

  useEffect(() => {
    const host = terminalHostRef.current;
    if (!host) return;
    let disposed = false;
    let frame = 0;
    // Type comes from the host's `code-block` text variant; xterm only accepts resolved values.
    const hostStyle = getComputedStyle(host);
    const hostFontSize = Number.parseFloat(hostStyle.fontSize);
    const options: ITerminalOptions = {
      theme: terminalTheme(host),
      fontFamily: hostStyle.fontFamily,
      cursorBlink: true,
      scrollback: 3_000,
      allowProposedApi: false,
    };
    // Unset keys keep xterm's default; an explicit undefined would clobber it.
    if (hostFontSize > 0) options.fontSize = hostFontSize;
    const terminal = new Terminal(options);
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host);
    terminalRef.current = terminal;
    const input = terminal.onData(onInput);
    const resize = terminal.onResize(({ cols, rows }) => onResize(cols, rows));
    const scheduleFit = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (disposed || host.clientWidth === 0 || host.clientHeight === 0) return;
        const dimensions = fit.proposeDimensions();
        if (!dimensions) return;
        terminal.resize(Math.max(20, Math.min(400, dimensions.cols)), Math.max(5, Math.min(200, dimensions.rows)));
        onResize(terminal.cols, terminal.rows);
      });
    };
    const observer = new ResizeObserver(scheduleFit);
    observer.observe(host);
    const themeObserver = new MutationObserver(() => {
      terminal.options.theme = terminalTheme(host);
    });
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style"] });
    scheduleFit();
    void document.fonts.ready.then(() => {
      if (!disposed) scheduleFit();
    });

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      themeObserver.disconnect();
      input.dispose();
      resize.dispose();
      terminalRef.current = null;
      terminal.dispose();
      host.replaceChildren();
    };
  }, [onInput, onResize]);

  const writeTerminal = useCallback((data: string) => terminalRef.current?.write(data), []);
  // Queue RIS with terminal bytes so a reset cannot be overtaken by pending writes.
  const resetTerminal = useCallback(() => terminalRef.current?.write("\u001bc\u001b[3J"), []);
  return { terminalHostRef, writeTerminal, resetTerminal };
};
