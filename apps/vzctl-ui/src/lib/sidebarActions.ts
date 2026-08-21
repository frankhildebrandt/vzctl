const handlers = new Map<string, () => void>();

/** Register a sidebar action handler; returns an unregister function. */
export function registerSidebarAction(
  id: string,
  handler: () => void,
): () => void {
  handlers.set(id, handler);
  return () => {
    if (handlers.get(id) === handler) handlers.delete(id);
  };
}

/** Invoke a registered sidebar action, if any. */
export function invokeSidebarAction(id: string): void {
  handlers.get(id)?.();
}
