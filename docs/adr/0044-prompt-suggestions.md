# Provider prompt suggestions

Claude launches with `--prompt-suggestions true`. Its `prompt_suggestion` event
becomes a provider-neutral `PromptSuggestion` event. Missing or unfamiliar payloads
are tolerated. Codex and Pi do not generate predictions.

The runner keeps only the latest prediction in memory and exposes it as optional
`promptSuggestion` in session status. New turns and driver disconnection clear it.
Predictions received during a turn are ignored. Suggestion events are excluded
from stored agent events. A change publishes a status cursor marker without the
prediction text; cursor pages attach the runner's current prediction to summary
payloads. Reconnection receives the current prediction, never a historical one.
The chat-list stream omits predictions; its idle summaries preserve the browser's
current prediction so the two streams cannot erase it by arrival order.
A runner restart forgets it. No migration or live runner replacement is required
to deserialize status from older runners, since the field is optional.

An empty, idle composer displays the prediction as its faint native placeholder.
Unmodified Tab inserts it as a local draft and places the caret at the end.
Typing hides the placeholder. Tab retains its normal behavior without a prediction,
with modifiers, while sending, or while composing an IME sequence.

The capability evidence and exact live payload are in
[the research note](../research/prompt-suggestions.md).
