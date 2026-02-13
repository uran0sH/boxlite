# BoxLite Restart Mechanism Design (Two-Layer Model with Flow Diagrams)

---

# 1. Architecture Overview

The restart system is divided into two layers:

---

## Level 1 — Persistent State Machine

Only stable states are persisted:

```
[Configured] --start()--> [Running] --stop()--> [Stopped]
      ^                         |
      |                         |
      |                         | exit (any reason)
      |                         v
      |---------------------- [Stopped]
```

No `Crashed`, `Restarting`, or `Failed` states exist.

Crash is treated as a runtime event that transitions to `Stopped`.
Permanent failure is indicated by `failure_reason` field, not a separate state.

---

## Level 2 — Runtime Control Flow

Transient runtime-only logic:

```
Process exit detected
        ↓
Classify exit reason
        ↓
Update persistent state → Stopped
        ↓
Evaluate restart policy
        ↓
    ├─ No restart → Done
    │
    └─ Restart allowed
            ↓
        Calculate backoff
            ↓
        Sleep(backoff)
            ↓
        Attempt start()
            ↓
        ├─ Success → Running
        └─ Failure → retry or Failed
```

Everything above exists only in memory.

---

# 2. Persistent State Definition

```rust
pub enum BoxStatus {
    Unknown,
    Configured,
    Running,
    Stopping,
    Stopped,
}
```

Exit reason is determined by `BoxState` fields:
- `last_exit_code == Some(0)`: Normal exit
- `last_exit_code == Some(!0)`: Error exit
- `last_exit_code == None`: Crashed (no exit code)
- `failure_reason == Some(...)`: Permanently failed (max restart attempts exceeded)

---

# 3. Runtime Crash Detection Flow

Monitoring task (only for boxes with restart policy):

```
Spawn monitoring task
        ↓
Loop every 5 seconds:
        ↓
Check is_process_alive(pid)
        ↓
    ├─ Alive → continue
    │
    └─ Dead → emit ExitDetected event
                    ↓
                BREAK monitor loop
```

Crash does NOT change state to Crashed.

Instead:

```
Running --exit detected--> Stopped (persist)
```

---

# 4. Exit Handling Flow

When exit is detected:

```
handle_exit(box_id, exit_code)
        ↓
Update:
  last_exit_code
  last_exit_reason
  last_stopped_at
        ↓
Persist state = Stopped
        ↓
Set failure_reason if max attempts exceeded
        ↓
Evaluate restart policy
```

---

# 5. Restart Decision Flow

```
Evaluate RestartPolicy
        ↓
Policy == Never?
        ├─ Yes → End
        └─ No
             ↓
Policy == OnFailure AND exit_code == 0?
        ├─ Yes → End
        └─ No
             ↓
Max attempts exceeded?
        ├─ Yes → Set failure_reason
        │         Persist Stopped
        │         End
        └─ No
             ↓
Schedule restart (runtime-only)
```

---

# 6. Restart Execution Flow (Runtime Layer)

```
Restart scheduled
        ↓
Calculate backoff delay:
  delay = min(base * 2^restart_count, cap)
        ↓
Sleep(delay)
        ↓
Attempt box.start()
        ↓
    ├─ Success
    │     ↓
    │  restart_count += 1
    │  last_started_at = now
    │  failure_reason = None
    │  Persist → Running
    │  Start monitoring again
    │
    └─ Failure
          ↓
       restart_count += 1
          ↓
       Set failure_reason
       Persist → Stopped
          ↓
       End (no retry - max exceeded)
```

Restarting is NOT persisted (runtime-only operation).

It is an in-memory phase.

---

# 7. Runtime Startup Recovery Flow

```
Runtime starts
        ↓
Load all boxes from database
        ↓
For each box:
        ↓
Check persisted status
```

---

### Case 1: Configured

```
Do nothing
```

---

### Case 2: Running

```
Check PID file
        ↓
    ├─ Missing → Treat as exit
    │             → Transition → Stopped
    │             → Evaluate restart policy
    │
    └─ Exists → Check process alive
            ↓
        ├─ Alive → Reattach monitor
        └─ Dead  → Treat as exit
                     → Transition → Stopped
                     → Evaluate restart policy
```

---

### Case 3: Stopped

```
restart_on_reboot?
        ↓
    ├─ false → Do nothing
    └─ true  → Evaluate restart policy
```

Note: If `failure_reason` is set (permanently failed), it will NOT auto-restart.

---

# 8. Restart Window (Flapping Detection)

To avoid restart storms:

```
if now - last_started_at > restart_window:
    restart_count = 0
```

Flow:

```
Before calculating backoff:
        ↓
Check restart window
        ↓
Reset counter if stable
```

This aligns with Kubernetes behavior.

---

# 9. Level 1 Transition Matrix

| Current    | Event        | Next    |
| ---------- | ------------ | ------- |
| Configured | start()      | Running |
| Running    | stop(manual) | Stopping |
| Running    | exit         | Stopped |
| Stopping   | complete     | Stopped |
| Stopped    | start()      | Running |

Level 2 events (exit, backoff, retry) never appear here.

---

# 10. Concurrency Model

```
Per-box mutex
        ↓
Exit handler acquires lock
        ↓
State persisted before restart attempt
        ↓
Only one restart attempt at a time
```

Monitoring task exits before scheduling restart.

---

# 11. Final Model Summary

### Level 1 — Durable Truth

```
Unknown
Configured
Running
Stopping
Stopped
```

Exit reason determined by `BoxState` fields:
- `last_exit_code`: Some(0) / Some(!0) / None
- `failure_reason`: Some(...) / None (permanent failure)
- `restart_count`: Number of restart attempts

### Level 2 — Runtime Logic

```
ExitDetected
BackoffTimer
RestartAttempt
FlappingReset
```

---

# 12. Why This Model Is Correct

* Crash is an event, not a state
* No ambiguous recovery states (Crashed, Restarting, Failed)
* Permanent failure indicated by fields, not state
* No partial restart persistence
* Deterministic reboot behavior
* Aligns with Kubernetes restart semantics
* Prevents restart duplication
* Clean separation: persistent states vs runtime logic
