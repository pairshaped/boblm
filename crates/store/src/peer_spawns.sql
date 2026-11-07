-- func: lookup
SELECT caller, arguments, resolved FROM peer_spawn_requests WHERE request_id = @request_id;

-- func: receipt
INSERT INTO peer_spawn_requests (request_id, caller, arguments, resolved) VALUES (@request_id, @caller, @arguments, @resolved);
