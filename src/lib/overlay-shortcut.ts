// src/lib/overlay-shortcut.ts

export function isOverlayDismissShortcut(
  event: Pick<
    KeyboardEvent,
    "altKey" | "ctrlKey" | "isComposing" | "key" | "metaKey" | "repeat" | "shiftKey"
  >
): boolean {
  if (event.repeat || event.isComposing) return false;
  if (event.key === "Escape") return true;

  return (
    event.key === " " &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.shiftKey
  );
}
