import { STEER_LINE_MESSAGE_TYPE } from "../engine/adapter-contract.js";

/** The line a Rerail message carries into a Pi or OMP compaction, so the summarizer never takes it for the person. */
export const RERAIL_COMPACTION_LABEL = "[Rerail: automated rule check sent to the agent, not a message from the user]";

interface MessageLike {
  role: string;
}

/**
 * The part of Pi's and OMP's `CompactionPreparation` the guard rewrites (pi-coding-agent 1.1.0 and 0.80.6
 * `compaction.d.ts`, OMP `pi-agent-core/src/compaction/compaction.ts`). Only OMP has `recentMessages`: the kept tail it
 * hands to remote compaction and the short summary.
 */
export interface CompactionPreparationLike<Message extends MessageLike> {
  messagesToSummarize: Message[];
  turnPrefixMessages: Message[];
  isSplitTurn: boolean;
  recentMessages?: Message[];
}

/** A message Rerail sent through the host's custom-message channel: a steer or the session-start note. */
function rerailMessage(message: MessageLike): boolean {
  return message.role === "custom"
    && "customType" in message
    && typeof message.customType === "string"
    && message.customType.startsWith("rerail.");
}

function textBlock(block: unknown): block is { type: "text"; text: string } {
  return typeof block === "object" && block !== null && "type" in block && block.type === "text" && "text" in block && typeof block.text === "string";
}

/** A copy of a Rerail message led by {@link RERAIL_COMPACTION_LABEL}; any other message as it is. */
function labelled<Message extends MessageLike>(message: Message): Message {
  if (!rerailMessage(message) || !("content" in message)) return message;
  const { content } = message;
  if (typeof content === "string") return { ...message, content: `${RERAIL_COMPACTION_LABEL}\n${content}` };
  if (!Array.isArray(content)) return message;
  const first = content.findIndex(textBlock);
  const blocks = first === -1
    ? [{ type: "text", text: RERAIL_COMPACTION_LABEL }, ...content]
    : content.with(first, { ...content[first], text: `${RERAIL_COMPACTION_LABEL}\n${content[first].text}` });
  return { ...message, content: blocks };
}

/**
 * What the host compacts once Rerail's messages are labelled, the person's steer lines left out (the summarizer reads
 * those steers in their tool results), and a split turn a Rerail message leads is folded back into the history. Both
 * hosts count every custom message as a turn start, so a steer can open the turn prefix, whose summary is anchored on an
 * "Original Request"; summarized with the history instead, the turn keeps the person's own request. OMP's
 * `recentMessages` runs parallel to the entry ids it keeps, so a steer line stays there, labelled like Rerail's other
 * messages. The messages themselves are shared with the session and are copied, never changed.
 */
function guarded<Message extends MessageLike>(preparation: CompactionPreparationLike<Message>): CompactionPreparationLike<Message> {
  const { messagesToSummarize, turnPrefixMessages, isSplitTurn, recentMessages } = preparation;
  const unsplit = isSplitTurn && turnPrefixMessages[0] !== undefined && rerailMessage(turnPrefixMessages[0]);
  const summarized = (messages: Message[]): Message[] => messages
    .filter(message => !(message.role === "custom" && "customType" in message && message.customType === STEER_LINE_MESSAGE_TYPE))
    .map(labelled);
  return {
    messagesToSummarize: summarized(unsplit ? [...messagesToSummarize, ...turnPrefixMessages] : messagesToSummarize),
    turnPrefixMessages: unsplit ? [] : summarized(turnPrefixMessages),
    isSplitTurn: isSplitTurn && !unsplit,
    ...(recentMessages === undefined ? {} : { recentMessages: recentMessages.map(labelled) }),
  };
}

/**
 * Pi's and OMP's `session_before_compact` handler: rewrites `preparation` in place, since both hosts go on to compact
 * the very object their handlers received, and returns nothing, so the host's own summarizer still writes the summary.
 * It never throws: a fault leaves one stderr line and the compaction as the host prepared it.
 */
export function guardCompaction<Message extends MessageLike>(preparation: CompactionPreparationLike<Message>): void {
  try {
    Object.assign(preparation, guarded(preparation));
  } catch (error) {
    process.stderr.write(`Rerail session_before_compact hook failed: ${error instanceof Error ? error.message : String(error)}\n`);
  }
}
