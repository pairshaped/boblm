const clock = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  minute: "2-digit",
});
const dated = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

/** "7:45 PM" for a message from today, "Oct 22, 9:23 AM" for an earlier day; no year, local time. */
export function messageTime(at: number, now: Date = new Date()): string {
  const when = new Date(at);
  const sameDay = when.toDateString() === now.toDateString();
  // Newer ICU puts a narrow no-break space before AM and PM; use an ordinary space.
  return (sameDay ? clock : dated).format(when).replace(/ /g, " ");
}
