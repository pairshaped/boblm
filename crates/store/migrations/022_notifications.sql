CREATE TABLE notifications (
 id TEXT NOT NULL PRIMARY KEY,
 session TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 category TEXT NOT NULL,
 title TEXT NOT NULL,
 message TEXT NOT NULL,
 question_id TEXT,
 state TEXT NOT NULL DEFAULT 'pending',
 reason TEXT NOT NULL DEFAULT '',
 created_at INTEGER NOT NULL
);
CREATE TABLE push_subscriptions (
 id TEXT NOT NULL PRIMARY KEY,
 endpoint TEXT NOT NULL UNIQUE,
 p256dh TEXT NOT NULL,
 auth TEXT NOT NULL,
 created_at INTEGER NOT NULL
);
CREATE TABLE push_deliveries (
 notification TEXT NOT NULL REFERENCES notifications(id) ON DELETE CASCADE,
 subscription TEXT NOT NULL REFERENCES push_subscriptions(id) ON DELETE CASCADE,
 state TEXT NOT NULL,
 PRIMARY KEY(notification, subscription)
);
