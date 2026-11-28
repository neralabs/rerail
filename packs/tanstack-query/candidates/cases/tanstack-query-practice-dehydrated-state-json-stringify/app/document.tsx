import { HydrationBoundary, QueryClient, dehydrate, type DehydratedState } from "@tanstack/react-query";
import Script from "next/script";
import { dehydrate as dehydrateStore } from "../src/store";
import { htmlEscapeJsonString } from "../src/escape";

type DocumentProps = { dehydratedState: DehydratedState; trpcState: unknown; children: React.ReactNode };

export function Document({ qc, children }: { qc: QueryClient; children: React.ReactNode }) {
  return (
    <html>
      <body>
        {children}
        <script
          dangerouslySetInnerHTML={{ __html: `window.__RQ__ = ${JSON.stringify(dehydrate(qc))}` }} // caught
        />
      </body>
    </html>
  );
}

export function StateScript(props: DocumentProps) {
  const json = JSON.stringify(props.dehydratedState); // caught
  return <script id="rq-state" type="application/json" dangerouslySetInnerHTML={{ __html: json }} />;
}

export function TrpcStateScript({ trpcState }: DocumentProps) {
  return <Script id="trpc-state">{`window.__TRPC_STATE__ = ${JSON.stringify(trpcState)}`}</Script>; // caught
}

export function Hydrated({ qc, children }: { qc: QueryClient; children: React.ReactNode }) {
  // The framework serialises the state; nothing here writes it into HTML.
  return <HydrationBoundary state={dehydrate(qc)}>{children}</HydrationBoundary>;
}

export function EscapedState(props: DocumentProps) {
  // Passed through an escaper before it reaches the HTML.
  return <script dangerouslySetInnerHTML={{ __html: htmlEscapeJsonString(JSON.stringify(props.dehydratedState)) }} />;
}

export function JsonLd({ data }: { data: Record<string, unknown> }) {
  // Not the query cache: structured data is another rule's concern.
  return <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: JSON.stringify(data) }} />;
}

export function StoreSnapshot() {
  // `dehydrate` here is the app's own store helper, not TanStack Query's.
  return <script dangerouslySetInnerHTML={{ __html: `window.__STORE__ = ${JSON.stringify(dehydrateStore())}` }} />;
}

export function StateInDataAttribute(props: DocumentProps) {
  // A JSX attribute value is escaped by React.
  return <div data-state={JSON.stringify(props.dehydratedState)} />;
}

export function StateDebugPanel({ dehydratedState, previous }: { dehydratedState: DehydratedState; previous: DehydratedState }) {
  // JSX text children and comparisons: React escapes the text, and a comparison writes nothing.
  const changed = JSON.stringify(dehydratedState) !== JSON.stringify(previous);
  return changed && <pre className="debug">{JSON.stringify(dehydratedState, null, 2)}</pre>;
}

export function stateLoaderScript(dehydratedState: DehydratedState) {
  // Script code, not markup: `i<keys.length` is a comparison, so nothing marks this template as HTML.
  return `var s=${JSON.stringify(dehydratedState)};for(var i=0;i<keys.length;i++){load(s[keys[i]])}`;
}
