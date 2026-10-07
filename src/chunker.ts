export interface StreamingSampleChunkerOptions {
  maxCharacters: number;
}

export interface FinishedStream {
  remainder: string;
  message: string;
}

interface StreamState {
  pending: string;
  message: string;
}

interface Boundary {
  contentEnd: number;
  consumedEnd: number;
}

function truncateStart(text: string, maximum: number): string {
  if (text.length <= maximum) return text;
  let start = text.length - maximum;
  const code = text.codePointAt(start)!;
  if (code >= 0xdc00 && code <= 0xdfff) start += 1;
  return text.slice(start);
}

function nextBoundary(text: string): Boundary | undefined {
  const newline = text.search(/\r?\n/u);
  const punctuationMatch = /[.!?](?=\s)/u.exec(text);
  const punctuation = punctuationMatch?.index ?? -1;

  if (newline === -1 && punctuation === -1) return undefined;
  if (newline !== -1 && (punctuation === -1 || newline < punctuation)) {
    const consumedEnd = text[newline] === "\r" && text[newline + 1] === "\n" ? newline + 2 : newline + 1;
    return { contentEnd: newline, consumedEnd };
  }
  return { contentEnd: punctuation + 1, consumedEnd: punctuation + 1 };
}

export class StreamingSampleChunker {
  readonly #maxCharacters: number;
  readonly #streams = new Map<string, StreamState>();

  constructor(options: StreamingSampleChunkerOptions) {
    if (!Number.isInteger(options.maxCharacters) || options.maxCharacters <= 0) {
      throw new RangeError("maxCharacters must be a positive integer");
    }
    this.#maxCharacters = options.maxCharacters;
  }

  append(streamId: string, delta: string): string[] {
    if (streamId.length === 0) throw new TypeError("streamId must not be empty");
    if (delta.length === 0) return [];

    const state = this.#streams.get(streamId) ?? { pending: "", message: "" };
    state.pending = truncateStart(state.pending + delta, this.#maxCharacters);
    state.message = truncateStart(state.message + delta, this.#maxCharacters);
    this.#streams.set(streamId, state);

    const completed: string[] = [];
    for (;;) {
      const boundary = nextBoundary(state.pending);
      if (boundary === undefined) break;
      const sentence = state.pending.slice(0, boundary.contentEnd).trim();
      state.pending = state.pending.slice(boundary.consumedEnd).trimStart();
      if (sentence.length > 0) completed.push(sentence);
    }
    return completed;
  }

  finish(streamId: string): FinishedStream | null {
    const state = this.#streams.get(streamId);
    if (state === undefined) return null;
    this.#streams.delete(streamId);
    const message = state.message.trim();
    if (message.length === 0) return null;
    return {
      remainder: state.pending.trim(),
      message,
    };
  }

  abort(streamId: string): void {
    this.#streams.delete(streamId);
  }

  clear(): void {
    this.#streams.clear();
  }
}
