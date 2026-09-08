export function mergeMessages<T extends { id: number }>(...groups: T[][]) {
  const messages = new Map<number, T>();
  groups.flat().forEach((message) => messages.set(message.id, message));
  return [...messages.values()].sort((left, right) => left.id - right.id);
}
