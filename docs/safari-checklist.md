# Safari composer check

Run this after the Web task provides a browser application. Record the Safari
version and device, then check both a desktop window and a phone viewport.

1. Focus the empty composer. Confirm the caret is visible and blinking.
2. Start a streaming response. Type text, select part of it, and insert more
   text while events append. Confirm exact text, selection, and focus survive.
3. Send with Enter. Confirm focus stays in the composer while the send is in
   flight. Shift+Enter must insert a newline. Check IME composition separately.
4. While within 80 px of the bottom, confirm new events keep the view at the
   bottom. Scroll farther up and confirm events preserve your reading position
   and show a new messages control. Use that control to return to the bottom.
5. Capture screenshots at desktop and phone widths for owner review of density,
   readability, and spacing. Record any failed step with its reproduction.

Linux checks do not verify Safari caret rendering. This checklist remains
manual until it is run on Safari.
