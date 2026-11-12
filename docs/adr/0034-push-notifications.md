# 0034: Owner-controlled Web Push

The app supports standards-based Web Push over the existing HTTPS deployment. The browser has a web
app manifest, PNG icons, and a push-only service worker. The worker doesn't cache application assets.
On iPhone, the owner installs the app on the Home Screen and enables notifications with a button in
the sidebar. Permission is requested only after that action. Locking the app unsubscribes that
browser; disabling notifications does the same without locking it.

Only a new question created through the `ask` MCP tool or a credential request through
`request_sops_secret` can produce a push notification.
There is no ad hoc notification tool. Updating, answering, deferring or deleting questions,
failures and completed turns do not push.

Question creation and its notification share one SQLite transaction. The durable notice ID
is `question:<question id>`, so identical retries cannot produce another notice.
Credential requests enqueue in their creation transaction with stable `secret:<request id>`
notice IDs, so retries cannot duplicate them. They use the same `questions` category and
config bool. Pending notices for retired questions are suppressed. With no push config, notices are
suppressed. The `questions` config bool defaults on. The sidebar bell subscribes or
unsubscribes the current browser, keeping on/off control on each device.

A visible, focused browser renews a short presence lease for the viewed chat. That suppresses pushes
for the chat across the owner's devices. The server checks presence and preferences again before
sending. Expired leases don't suppress anything. The worker always displays received pushes because
Safari requires visible notifications. Clicking a notification opens its chat and, for a question,
opens the Questions modal and focuses the question through `?question=<id>`.

Subscriptions require the owner's bearer token. The server accepts HTTPS endpoints on known browser
push services and validates the encryption keys before storing them. Payloads are encrypted with
`web-push-native` and signed with the server's P-256 VAPID key. No credential value or SOPS path is sent
in a notification. The push service sees the destination and encrypted bytes. A notification's title
and body can appear on a device's lock screen, so agents must keep secrets out of them.

Each notification/subscription pair is claimed durably before its HTTP request. A crashed or uncertain
send is held and never automatically replayed. This trades a possible missed push for avoiding duplicate
pushes after an uncertain request. Service-worker notification tags also use the durable notice ID.
Expired subscriptions (HTTP 404 or 410) are removed. Other failed sends remain recorded. A separate
background worker prevents a slow push service from delaying scheduled prompts and question answers.
Deleting a chat removes its notification records and delivery receipts.

## Server configuration

Generate a stable P-256 key outside the checkout and protect it:

```sh
umask 077
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out ~/.config/boblm/push-key.pem
chmod 600 ~/.config/boblm/push-key.pem
```

Add `notifications` to the existing server JSON, using an absolute file path and your contact address:

```json
{
  "notifications": {
    "privateKeyFile": "/home/user/.config/boblm/push-key.pem",
    "subject": "mailto:you@example.com",
    "questions": true
  }
}
```

The legacy `agentNotices`, `completion` and `failures` keys remain accepted as deprecated,
ignored no-ops, so existing configuration files still parse.

Keep the key stable: subscriptions are tied to its public key. After rotating it, disable and reenable
notifications on each device. Configuration takes effect after the next authorized server deployment.
There is no settings screen. For a dev instance, use its own config and key.

The implementation follows [WebKit's Home Screen push requirements](https://webkit.org/blog/13878/)
and [Apple's Web Push guidance](https://developer.apple.com/documentation/usernotifications/sending-web-push-notifications-in-web-apps-and-browsers).
Actual iPhone installation, permission, and locked-screen delivery require an owner check on the device.
