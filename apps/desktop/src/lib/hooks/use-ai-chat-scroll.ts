import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { AiChatMessage } from "./../aspector/chat/types";

/**
 * Owns the chat thread's scroll behavior: keeps the view pinned to the latest
 * message while the user is already at the bottom, surfaces a "scroll to latest"
 * affordance once they scroll up, and re-pins on session switch.
 *
 * Returns the scroll container ref plus the handlers the panel wires to the DOM
 * (`handleBodyScroll`, `scrollToBottom`) and a `pinToBottom()` the send path calls
 * to force the next streamed content to stay in view.
 */
export function useAiChatScroll({
  messages,
  activeSessionId,
  revealKey,
}: {
  messages: AiChatMessage[];
  activeSessionId: string | null;
  /**
   * Changes whenever a blocking card (AskUser question / proposed plan) appears
   * below the thread. Those live in a separate store, so a `messages`-keyed scroll
   * never fires for them — this force-reveals the card instead of letting it slip
   * below the fold (under the composer).
   */
  revealKey?: string | null;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const pinnedToBottomRef = useRef(true);
  const [showScrollDown, setShowScrollDown] = useState(false);

  const pinToBottom = useCallback(() => {
    pinnedToBottomRef.current = true;
    setShowScrollDown(false);
  }, []);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "auto") => {
    const element = scrollRef.current;
    if (!element) return;
    pinnedToBottomRef.current = true;
    setShowScrollDown(false);
    element.scrollTo({ top: element.scrollHeight, behavior });
  }, []);

  const handleBodyScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    const distanceFromBottom = element.scrollHeight - element.scrollTop - element.clientHeight;
    const pinned = distanceFromBottom <= 28;
    pinnedToBottomRef.current = pinned;
    setShowScrollDown(!pinned && element.scrollHeight - element.clientHeight > 48);
  }, []);

  // Keep the view pinned to the latest message while the user is already at the bottom.
  // When they have scrolled up to read, new content streams in without yanking the viewport.
  useLayoutEffect(() => {
    if (!pinnedToBottomRef.current) return;
    const element = scrollRef.current;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
  }, [messages]);

  useEffect(() => {
    pinnedToBottomRef.current = true;
    setShowScrollDown(false);
    const element = scrollRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [activeSessionId]);

  // A blocking interaction card just appeared — force it into view (overriding a
  // scrolled-up position), since it requires the user's response.
  useLayoutEffect(() => {
    if (!revealKey) return;
    const element = scrollRef.current;
    if (!element) return;
    pinnedToBottomRef.current = true;
    setShowScrollDown(false);
    element.scrollTop = element.scrollHeight;
  }, [revealKey]);

  return { scrollRef, showScrollDown, scrollToBottom, handleBodyScroll, pinToBottom };
}
