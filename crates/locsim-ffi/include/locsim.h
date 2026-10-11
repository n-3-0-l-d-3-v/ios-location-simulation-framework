/*
 * locsim.h - C interface of locsim-ffi: one supervised simulation session.
 *
 * A session produces synthetic location samples from a scenario document,
 * supervised by a health policy. The library reads no clock, no file and no
 * device: every time is passed in by the caller. It is not a location
 * provider for any platform and it makes no location appear anywhere; it is
 * the boundary a platform adapter can call.
 *
 * VERSION
 *   LOCSIM_ABI_VERSION is the version this header describes;
 *   locsim_abi_version() returns the version of the library that was
 *   linked. Any change to the interface, of any kind, is a new number.
 *   Compare the two for equality before anything else and do not continue
 *   if they differ.
 *
 * HANDLES
 *   A session is named by a locsim_session_t, a 64-bit integer, not a
 *   pointer. 0 is never a valid handle. A handle is never issued twice: one
 *   that was destroyed, or never issued, is refused with
 *   LOCSIM_ERR_INVALID_HANDLE by every function. Passing such a handle is
 *   an error the library reports, not undefined behaviour.
 *
 * STATUS
 *   Every function but locsim_abi_version returns a locsim_status_t.
 *   LOCSIM_OK is 0; everything else is a refusal or a failure.
 *
 * ORDER OF CHECKS
 *   Always the same: null pointers; then struct_size; then the other
 *   arguments in the order of the parameter list; then the handle; then
 *   the state of the session; then the operation itself.
 *
 * POINTERS - what the caller must guarantee
 *   The library checks what can be checked: null pointers, lengths, sizes,
 *   codes, UTF-8. It cannot tell whether a pointer that is not null points
 *   at memory that exists, is large enough or is still alive. That is the
 *   caller's responsibility, and getting it wrong is undefined behaviour
 *   that the library neither detects nor recovers from.
 *
 *   - Null. A pointer may be null only where its description says so. A
 *     null pointer anywhere else is LOCSIM_ERR_NULL_POINTER, and that call
 *     writes nothing at all.
 *   - Extent. Every pointer is to memory readable (inputs) or writable
 *     (outputs) for the full size of its type, or for the number of bytes
 *     given beside it.
 *   - Alignment. Every pointer is suitably aligned for its type (8 bytes
 *     for the structs). Byte buffers need no alignment.
 *   - Lifetime. Memory passed to a call is valid until the call returns
 *     and is not written by another thread during it. The library keeps no
 *     pointer after returning and never frees or takes ownership of
 *     anything the caller passes. Nothing the library allocates is handed
 *     to the caller: the only thing to release is a session, with
 *     locsim_session_destroy.
 *   - Overlap. Memory a call writes overlaps neither other memory the same
 *     call writes nor memory it reads.
 *   - Lengths and capacities are in bytes. No terminator is read, written
 *     or counted.
 *
 * OUTPUTS - what a call writes
 *   A call that does not return LOCSIM_OK writes nothing. The one
 *   exception is locsim_session_create, described there.
 *   A call that returns LOCSIM_OK writes every output in full, except that
 *   where an output has a flag beside it (has_sample, has_deadline,
 *   has_event) and the flag is written as 0, the output itself is not
 *   written.
 *   Output structs carry struct_size, which the caller sets to
 *   sizeof(the struct) before the call. Any other value is
 *   LOCSIM_ERR_STRUCT_SIZE.
 *
 * TIME
 *   Every now_ns is a count of nanoseconds on a time scale the caller
 *   chooses. Where the count starts does not matter to the library: it
 *   only compares times and subtracts them, and any int64_t is accepted,
 *   negative values included. The timestamps of samples, deadlines, health
 *   reports and events are on the same scale as the times passed in. The
 *   times of one session mean nothing to another.
 *   The caller should pass times that do not decrease within a session.
 *   The library does not enforce this. A time earlier than one already
 *   given is recorded once as LOCSIM_EVENT_CLOCK_WENT_BACKWARDS; a resume
 *   earlier than its pause is refused.
 *   A time that cannot be represented is never produced: a run whose next
 *   sample would fall after INT64_MAX fails with
 *   LOCSIM_ERR_SCHEDULE_OVERFLOW.
 *
 * THREADS
 *   Any function may be called from any thread at any time, including
 *   locsim_session_destroy while another thread is using the same session.
 *   Calls on different sessions do not affect one another.
 *   Two calls on the same session never run together and never wait for
 *   one another: one of them proceeds and the other returns
 *   LOCSIM_ERR_BUSY at once, having done nothing and written nothing. Call
 *   again later. This includes locsim_session_destroy: LOCSIM_ERR_BUSY
 *   means the session was not destroyed and its handle is still valid.
 *   A call that was told LOCSIM_ERR_BUSY because a destroy was in progress
 *   is told LOCSIM_ERR_INVALID_HANDLE the next time.
 *
 * PANICS
 *   A failure inside the library does not unwind into the caller. The call
 *   returns LOCSIM_ERR_PANIC; the session can no longer be used and every
 *   later call on it returns LOCSIM_ERR_POISONED until it is destroyed,
 *   which still succeeds. Other sessions are not affected. This holds for
 *   a library built to unwind; one built to abort on panic ends the
 *   process instead, as do stack exhaustion and allocation failure.
 */
#ifndef LOCSIM_H
#define LOCSIM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Define LOCSIM_DLL before including this header to import the functions
 * from a Windows DLL explicitly. It is never required. */
#if defined(_WIN32) && defined(LOCSIM_DLL)
#define LOCSIM_API __declspec(dllimport)
#else
#define LOCSIM_API
#endif

#define LOCSIM_ABI_VERSION 1u

/* Limits. */
#define LOCSIM_SCENARIO_MAX_BYTES 16777216u    /* longest scenario document */
#define LOCSIM_CREATE_ERROR_TEXT_MAX 4096u     /* longest text create reports */
#define LOCSIM_EVENT_CAPACITY_MAX 4096u        /* largest event queue */
#define LOCSIM_SESSION_LIMIT 1024u             /* most sessions at one time */

typedef uint64_t locsim_session_t;
typedef int32_t locsim_status_t;

/* ---- Status codes ------------------------------------------------------ */

#define LOCSIM_OK 0

/* The call itself was not acceptable. */
#define LOCSIM_ERR_NULL_POINTER 1
#define LOCSIM_ERR_INVALID_HANDLE 2
#define LOCSIM_ERR_INVALID_UTF8 3
#define LOCSIM_ERR_INVALID_ARGUMENT 4     /* a length, capacity or code */
#define LOCSIM_ERR_STRUCT_SIZE 5
#define LOCSIM_ERR_BUSY 6                 /* another call is using the session */
#define LOCSIM_ERR_LIMIT 7                /* LOCSIM_SESSION_LIMIT sessions exist */

/* The session could not be built. */
#define LOCSIM_ERR_SCENARIO 20
#define LOCSIM_ERR_POLICY 21

/* The simulation refused a call or a run failed. Only the kind of the
 * error crosses the boundary; nothing else it carried does. */
#define LOCSIM_ERR_PROVIDER_INVALID_SCENARIO 30
#define LOCSIM_ERR_PROVIDER_TRANSITION 31 /* not allowed in this state */
#define LOCSIM_ERR_PROVIDER_MOVEMENT 33
#define LOCSIM_ERR_PROVIDER_NOISE 34
#define LOCSIM_ERR_PROVIDER_DERIVATION 35
#define LOCSIM_ERR_PROVIDER_VALIDATION 36 /* a sample was rejected or withheld */
#define LOCSIM_ERR_SCHEDULE_INVALID_INTERVAL 40
#define LOCSIM_ERR_SCHEDULE_OVERFLOW 41   /* a time cannot be represented */
#define LOCSIM_ERR_SCHEDULE_ALREADY_PAUSED 42
#define LOCSIM_ERR_SCHEDULE_NOT_PAUSED 43
#define LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS 44

/* The library itself failed. */
#define LOCSIM_ERR_PANIC 90
#define LOCSIM_ERR_POISONED 91

/* ---- Values of fields --------------------------------------------------- */

/* The state of a session or of a run. */
#define LOCSIM_STATE_IDLE 0
#define LOCSIM_STATE_STARTING 1
#define LOCSIM_STATE_RUNNING 2
#define LOCSIM_STATE_PAUSED 3
#define LOCSIM_STATE_STOPPING 4
#define LOCSIM_STATE_ERROR 5
#define LOCSIM_STATE_RECOVERING 6

/* What produced a sample. */
#define LOCSIM_SOURCE_SIMULATION 0
#define LOCSIM_SOURCE_REPLAY 1
#define LOCSIM_SOURCE_TEST 2

/* locsim_health_t.health */
#define LOCSIM_HEALTH_HEALTHY 0
#define LOCSIM_HEALTH_DEGRADED 1   /* a run exists and a cause is active */
#define LOCSIM_HEALTH_RECOVERING 2 /* a run failed; a restart is scheduled */
#define LOCSIM_HEALTH_FAILED 3     /* a run failed; nothing more will be tried */
#define LOCSIM_HEALTH_STOPPED 4    /* no run exists and none is scheduled */

/* locsim_health_t.emission */
#define LOCSIM_EMISSION_NOT_EXPECTED 0 /* nothing is running, or it is paused */
#define LOCSIM_EMISSION_FLOWING 1
#define LOCSIM_EMISSION_STALLED 2      /* running, and no sample is arriving */

/* locsim_health_t.causes, as bits */
#define LOCSIM_CAUSE_STALLED 1
#define LOCSIM_CAUSE_FALLING_BEHIND 2
#define LOCSIM_CAUSE_PROBATION 4       /* restarted, and not yet stable */
#define LOCSIM_CAUSE_EXTERNAL_FAULT 8

/* The components a fault can be reported for. A component's bit in
 * locsim_health_t.active_faults is 1 << (component - 1). */
#define LOCSIM_COMPONENT_STORE 1
#define LOCSIM_COMPONENT_ADAPTER 2
#define LOCSIM_COMPONENT_DELIVERY 3

/* locsim_event_t.level */
#define LOCSIM_LEVEL_ERROR 0
#define LOCSIM_LEVEL_WARN 1
#define LOCSIM_LEVEL_INFO 2
#define LOCSIM_LEVEL_DEBUG 3

/* locsim_event_t.kind. After each, what arg[] holds; what is not named is
 * 0. Counts that do not fit an int64_t are INT64_MAX. */
#define LOCSIM_EVENT_RUN_STARTED 1          /* arg[0] 1 if by a recovery; arg[1] its attempt */
#define LOCSIM_EVENT_RUN_ENDED 2            /* samples, withheld, missed ticks; arg[3] 0 failed, 1 stopped */
#define LOCSIM_EVENT_STOPPED 3
#define LOCSIM_EVENT_PAUSED 4
#define LOCSIM_EVENT_RESUMED 5
#define LOCSIM_EVENT_OPERATION_REJECTED 6   /* arg[0] 0 start, 1 stop, 2 pause, 3 resume; error set */
#define LOCSIM_EVENT_RUN_FAILED 7           /* error set */
#define LOCSIM_EVENT_RECOVERY_SCHEDULED 8   /* arg[0] attempt; arg[1] when it is due */
#define LOCSIM_EVENT_RECOVERY_ATTEMPTED 9   /* arg[0] attempt */
#define LOCSIM_EVENT_RECOVERY_START_FAILED 10 /* arg[0] attempt; error set */
#define LOCSIM_EVENT_RECOVERED 11           /* arg[0] attempts it took */
#define LOCSIM_EVENT_FAILED 12              /* arg[0] 0 attempts exhausted, 1 permanent error, 2 retry time not representable */
#define LOCSIM_EVENT_STALLED 13             /* arg[0] nanoseconds without a sample */
#define LOCSIM_EVENT_STALL_CLEARED 14
#define LOCSIM_EVENT_FALLING_BEHIND 15      /* arg[0] ticks missed; arg[1] slots of the window */
#define LOCSIM_EVENT_CAUGHT_UP 16
#define LOCSIM_EVENT_FAULT_REPORTED 17      /* arg[0] component */
#define LOCSIM_EVENT_FAULT_CLEARED 18       /* arg[0] component */
#define LOCSIM_EVENT_HEALTH_CHANGED 19      /* arg[0] health before; arg[1] after */
#define LOCSIM_EVENT_CLOCK_WENT_BACKWARDS 20 /* arg[0] latest time given before; at_ns the earlier one */

/* ---- Structs ------------------------------------------------------------ */

/* The health policy. Input. 64 bytes, aligned to 8, no padding. Values the
 * policy does not allow are LOCSIM_ERR_POLICY, with an explanation. */
typedef struct locsim_policy {
  uint32_t struct_size;            /* sizeof(locsim_policy_t) */
  uint32_t max_recovery_attempts;  /* restarts allowed per failure episode; 0: a failure is final */
  double backoff_initial_s;        /* delay before the first restart; >= 0 */
  double backoff_multiplier;       /* applied after each attempt; >= 1 */
  double backoff_max_s;            /* upper limit of the delay; >= backoff_initial_s */
  double stable_after_s;           /* how long a restarted run must emit before the episode is over */
  double stall_after_s;            /* silence longer than this is a stall; longer than the scenario's update interval */
  uint64_t missed_window_ticks;    /* length of the window missed ticks are judged over; >= 1 */
  uint64_t max_missed_in_window;   /* more missed ticks than this in a window is falling behind */
} locsim_policy_t;

/* One emitted sample. Output. 88 bytes, aligned to 8, no padding. */
typedef struct locsim_sample {
  uint32_t struct_size;       /* set by the caller: sizeof(locsim_sample_t) */
  uint32_t source;            /* LOCSIM_SOURCE_* */
  uint32_t simulation_state;  /* LOCSIM_STATE_* */
  uint8_t has_speed;          /* 0: speed_mps is not known, and is NaN */
  uint8_t has_course;         /* 0: course_deg is not known, and is NaN */
  uint8_t reserved[2];        /* 0 */
  int64_t timestamp_ns;       /* the time of the sample's tick, on the caller's scale */
  uint64_t run;               /* the run it belongs to. A change means a new run:
                                 positions start again and nothing connects the
                                 samples before the change to those after it. */
  double latitude_deg;
  double longitude_deg;
  double altitude_m;
  double horizontal_accuracy_m;
  double vertical_accuracy_m;
  double speed_mps;           /* mean ground speed since the previous sample */
  double course_deg;          /* direction of travel, degrees clockwise from true north, [0, 360) */
} locsim_sample_t;

/* The state of a session. Output. 192 bytes, aligned to 8, no padding
 * other than reserved. A time is meaningful only when its has_ flag is 1
 * and is 0 otherwise. */
typedef struct locsim_health {
  uint32_t struct_size;          /* set by the caller: sizeof(locsim_health_t) */
  uint32_t lifecycle;            /* what the session is doing: LOCSIM_STATE_* */
  uint32_t health;               /* one word for the whole: LOCSIM_HEALTH_* */
  uint32_t emission;             /* whether samples are arriving: LOCSIM_EMISSION_* */
  uint32_t causes;               /* LOCSIM_CAUSE_* bits; 0 unless health is degraded */
  uint32_t active_faults;        /* one bit per component with an uncleared fault */
  uint32_t attempts_used;        /* restart attempts used in this failure episode */
  uint32_t max_attempts;
  uint32_t last_error;           /* status of the error that ended the latest failed run or restart; 0 if none */
  uint32_t current_state;        /* the state of the current run: LOCSIM_STATE_* */
  uint8_t has_observed_at;
  uint8_t has_retry_at;
  uint8_t has_last_sample_time;
  uint8_t trajectory_complete;   /* 1: the trajectory has reached its end */
  uint8_t reserved[4];           /* 0 */
  int64_t observed_at_ns;        /* the latest time passed to any call */
  int64_t retry_at_ns;           /* when the next restart attempt is due */
  int64_t silent_for_ns;         /* meaningful when emission is LOCSIM_EMISSION_STALLED */
  int64_t last_sample_time_ns;
  uint64_t run;                  /* number of the current or latest run; 0 before any */
  uint64_t current_sample_count; /* of the current run */
  uint64_t current_failed_count;
  uint64_t current_missed_ticks;
  uint64_t total_runs_started;   /* totals: over the whole life of the session */
  uint64_t total_runs_ended;
  uint64_t total_samples;
  uint64_t total_withheld;
  uint64_t total_missed_ticks;
  uint64_t total_failures;
  uint64_t total_recovery_attempts;
  uint64_t total_recoveries_completed;
  uint64_t total_terminal_failures;
  uint64_t total_rejected_operations;
} locsim_health_t;

/* One thing that happened to a session. Output. 64 bytes, aligned to 8, no
 * padding. An event holds times, counts and codes: no text and no position. */
typedef struct locsim_event {
  uint32_t struct_size;  /* set by the caller: sizeof(locsim_event_t) */
  uint32_t level;        /* LOCSIM_LEVEL_* */
  uint32_t kind;         /* LOCSIM_EVENT_* */
  uint32_t error;        /* the status of the error the event carries, or 0 */
  int64_t at_ns;         /* the time passed to the call during which it happened */
  uint64_t run;          /* the run it concerns; 0 before any run */
  int64_t arg[4];        /* by kind; see LOCSIM_EVENT_* */
} locsim_event_t;

/* ---- Layout, checked by the compiler ------------------------------------ */

#if defined(__cplusplus)
#define LOCSIM_ASSERT(condition) static_assert(condition, #condition)
#define LOCSIM_ALIGNOF(type) alignof(type)
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
#define LOCSIM_ASSERT(condition) _Static_assert(condition, #condition)
#define LOCSIM_ALIGNOF(type) _Alignof(type)
#endif

#ifdef LOCSIM_ASSERT
LOCSIM_ASSERT(sizeof(locsim_session_t) == 8);
LOCSIM_ASSERT(sizeof(locsim_status_t) == 4);
LOCSIM_ASSERT(sizeof(double) == 8);

LOCSIM_ASSERT(sizeof(locsim_policy_t) == 64);
LOCSIM_ASSERT(LOCSIM_ALIGNOF(locsim_policy_t) == 8);
LOCSIM_ASSERT(offsetof(locsim_policy_t, struct_size) == 0);
LOCSIM_ASSERT(offsetof(locsim_policy_t, max_recovery_attempts) == 4);
LOCSIM_ASSERT(offsetof(locsim_policy_t, backoff_initial_s) == 8);
LOCSIM_ASSERT(offsetof(locsim_policy_t, backoff_multiplier) == 16);
LOCSIM_ASSERT(offsetof(locsim_policy_t, backoff_max_s) == 24);
LOCSIM_ASSERT(offsetof(locsim_policy_t, stable_after_s) == 32);
LOCSIM_ASSERT(offsetof(locsim_policy_t, stall_after_s) == 40);
LOCSIM_ASSERT(offsetof(locsim_policy_t, missed_window_ticks) == 48);
LOCSIM_ASSERT(offsetof(locsim_policy_t, max_missed_in_window) == 56);

LOCSIM_ASSERT(sizeof(locsim_sample_t) == 88);
LOCSIM_ASSERT(LOCSIM_ALIGNOF(locsim_sample_t) == 8);
LOCSIM_ASSERT(offsetof(locsim_sample_t, struct_size) == 0);
LOCSIM_ASSERT(offsetof(locsim_sample_t, source) == 4);
LOCSIM_ASSERT(offsetof(locsim_sample_t, simulation_state) == 8);
LOCSIM_ASSERT(offsetof(locsim_sample_t, has_speed) == 12);
LOCSIM_ASSERT(offsetof(locsim_sample_t, has_course) == 13);
LOCSIM_ASSERT(offsetof(locsim_sample_t, reserved) == 14);
LOCSIM_ASSERT(offsetof(locsim_sample_t, timestamp_ns) == 16);
LOCSIM_ASSERT(offsetof(locsim_sample_t, run) == 24);
LOCSIM_ASSERT(offsetof(locsim_sample_t, latitude_deg) == 32);
LOCSIM_ASSERT(offsetof(locsim_sample_t, longitude_deg) == 40);
LOCSIM_ASSERT(offsetof(locsim_sample_t, altitude_m) == 48);
LOCSIM_ASSERT(offsetof(locsim_sample_t, horizontal_accuracy_m) == 56);
LOCSIM_ASSERT(offsetof(locsim_sample_t, vertical_accuracy_m) == 64);
LOCSIM_ASSERT(offsetof(locsim_sample_t, speed_mps) == 72);
LOCSIM_ASSERT(offsetof(locsim_sample_t, course_deg) == 80);

LOCSIM_ASSERT(sizeof(locsim_health_t) == 192);
LOCSIM_ASSERT(LOCSIM_ALIGNOF(locsim_health_t) == 8);
LOCSIM_ASSERT(offsetof(locsim_health_t, struct_size) == 0);
LOCSIM_ASSERT(offsetof(locsim_health_t, lifecycle) == 4);
LOCSIM_ASSERT(offsetof(locsim_health_t, health) == 8);
LOCSIM_ASSERT(offsetof(locsim_health_t, emission) == 12);
LOCSIM_ASSERT(offsetof(locsim_health_t, causes) == 16);
LOCSIM_ASSERT(offsetof(locsim_health_t, active_faults) == 20);
LOCSIM_ASSERT(offsetof(locsim_health_t, attempts_used) == 24);
LOCSIM_ASSERT(offsetof(locsim_health_t, max_attempts) == 28);
LOCSIM_ASSERT(offsetof(locsim_health_t, last_error) == 32);
LOCSIM_ASSERT(offsetof(locsim_health_t, current_state) == 36);
LOCSIM_ASSERT(offsetof(locsim_health_t, has_observed_at) == 40);
LOCSIM_ASSERT(offsetof(locsim_health_t, has_retry_at) == 41);
LOCSIM_ASSERT(offsetof(locsim_health_t, has_last_sample_time) == 42);
LOCSIM_ASSERT(offsetof(locsim_health_t, trajectory_complete) == 43);
LOCSIM_ASSERT(offsetof(locsim_health_t, reserved) == 44);
LOCSIM_ASSERT(offsetof(locsim_health_t, observed_at_ns) == 48);
LOCSIM_ASSERT(offsetof(locsim_health_t, retry_at_ns) == 56);
LOCSIM_ASSERT(offsetof(locsim_health_t, silent_for_ns) == 64);
LOCSIM_ASSERT(offsetof(locsim_health_t, last_sample_time_ns) == 72);
LOCSIM_ASSERT(offsetof(locsim_health_t, run) == 80);
LOCSIM_ASSERT(offsetof(locsim_health_t, current_sample_count) == 88);
LOCSIM_ASSERT(offsetof(locsim_health_t, current_failed_count) == 96);
LOCSIM_ASSERT(offsetof(locsim_health_t, current_missed_ticks) == 104);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_runs_started) == 112);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_runs_ended) == 120);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_samples) == 128);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_withheld) == 136);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_missed_ticks) == 144);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_failures) == 152);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_recovery_attempts) == 160);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_recoveries_completed) == 168);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_terminal_failures) == 176);
LOCSIM_ASSERT(offsetof(locsim_health_t, total_rejected_operations) == 184);

LOCSIM_ASSERT(sizeof(locsim_event_t) == 64);
LOCSIM_ASSERT(LOCSIM_ALIGNOF(locsim_event_t) == 8);
LOCSIM_ASSERT(offsetof(locsim_event_t, struct_size) == 0);
LOCSIM_ASSERT(offsetof(locsim_event_t, level) == 4);
LOCSIM_ASSERT(offsetof(locsim_event_t, kind) == 8);
LOCSIM_ASSERT(offsetof(locsim_event_t, error) == 12);
LOCSIM_ASSERT(offsetof(locsim_event_t, at_ns) == 16);
LOCSIM_ASSERT(offsetof(locsim_event_t, run) == 24);
LOCSIM_ASSERT(offsetof(locsim_event_t, arg) == 32);
#endif

/* ---- Functions ---------------------------------------------------------- */

/* The version of the library that was linked. Compare with
 * LOCSIM_ABI_VERSION. */
LOCSIM_API uint32_t locsim_abi_version(void);

/*
 * Builds a session from a scenario document and a health policy. The new
 * session is stopped.
 *
 *   scenario_json   The document: scenario_len bytes of UTF-8 JSON. May be
 *                   null only if scenario_len is 0. At most
 *                   LOCSIM_SCENARIO_MAX_BYTES, else LOCSIM_ERR_INVALID_ARGUMENT.
 *   policy          Not null. Read only.
 *   event_capacity  Length of the session's event queue, 1 to
 *                   LOCSIM_EVENT_CAPACITY_MAX, else LOCSIM_ERR_INVALID_ARGUMENT.
 *   out_session     Not null. Receives the handle.
 *   err_buf         Room for an explanation: err_cap bytes. May be null
 *                   only if err_cap is 0.
 *   out_err_len     Not null. Receives the length of the explanation.
 *
 * What is written, by status:
 *
 *   LOCSIM_OK                 *out_session = the handle; *out_err_len = 0;
 *                             err_buf untouched.
 *   LOCSIM_ERR_NULL_POINTER   nothing at all.
 *   LOCSIM_ERR_SCENARIO,      *out_session = 0; *out_err_len = length of
 *   LOCSIM_ERR_POLICY         the text. If err_cap >= that length, the whole
 *                             text is written to err_buf and the bytes after
 *                             it are untouched; otherwise err_buf is
 *                             untouched. Never part of the text.
 *   any other status          *out_session = 0; *out_err_len = 0; err_buf
 *                             untouched.
 *
 * The text is UTF-8, one line per problem, at most
 * LOCSIM_CREATE_ERROR_TEXT_MAX bytes (cut at a character if longer), with
 * no terminator. It can repeat values of the document. A call that fails
 * creates nothing, so it can be repeated with a larger buffer.
 *
 * Other statuses: LOCSIM_ERR_STRUCT_SIZE, LOCSIM_ERR_INVALID_UTF8,
 * LOCSIM_ERR_LIMIT, LOCSIM_ERR_PANIC.
 */
LOCSIM_API locsim_status_t locsim_session_create(
    const uint8_t *scenario_json, size_t scenario_len,
    const locsim_policy_t *policy, uint32_t event_capacity,
    locsim_session_t *out_session,
    uint8_t *err_buf, size_t err_cap, size_t *out_err_len);

/*
 * Ends a session and invalidates its handle.
 *
 *   LOCSIM_OK                  The session is gone.
 *   LOCSIM_ERR_INVALID_HANDLE  Never issued, or already destroyed.
 *   LOCSIM_ERR_BUSY            Another call is using the session. Nothing
 *                              was done; the handle is still valid.
 *   LOCSIM_ERR_PANIC           The handle was invalidated and releasing the
 *                              session failed. Destroying again is harmless.
 */
LOCSIM_API locsim_status_t locsim_session_destroy(locsim_session_t session);

/* Begins a run whose first sample is stamped now_ns. The session must be
 * stopped, else LOCSIM_ERR_PROVIDER_TRANSITION. */
LOCSIM_API locsim_status_t locsim_session_start(locsim_session_t session, int64_t now_ns);

/* Ends the session's run, whatever state it is in, and cancels a pending
 * restart. LOCSIM_ERR_PROVIDER_TRANSITION if the session is stopped. */
LOCSIM_API locsim_status_t locsim_session_stop(locsim_session_t session);

/* Pauses a running session, else LOCSIM_ERR_PROVIDER_TRANSITION. */
LOCSIM_API locsim_status_t locsim_session_pause(locsim_session_t session, int64_t now_ns);

/* Resumes a paused session, else LOCSIM_ERR_PROVIDER_TRANSITION. A now_ns
 * earlier than that of the pause is
 * LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS and the session stays paused. */
LOCSIM_API locsim_status_t locsim_session_resume(locsim_session_t session, int64_t now_ns);

/*
 * Asks for the sample due at now_ns.
 *
 *   out_sample      Not null; struct_size set.
 *   out_has_sample  Not null.
 *
 *   LOCSIM_OK, *out_has_sample == 1   *out_sample is the sample.
 *   LOCSIM_OK, *out_has_sample == 0   Nothing is due, or the session is not
 *                                     running. *out_sample is not written.
 *   a LOCSIM_ERR_PROVIDER_* or        The run has just failed; nothing was
 *   LOCSIM_ERR_SCHEDULE_* status      emitted and nothing is written. It is
 *                                     reported by this one call: later calls
 *                                     return LOCSIM_OK without a sample until
 *                                     a run is running again.
 *
 * If a restart attempt is due it is made first and the new run is polled
 * in the same call.
 */
LOCSIM_API locsim_status_t locsim_session_poll(
    locsim_session_t session, int64_t now_ns,
    locsim_sample_t *out_sample, uint8_t *out_has_sample);

/*
 * Says when the session next needs a call: the earlier of the time the
 * next sample is due (only while running) and the time of a pending
 * restart attempt (only while recovering).
 *
 *   out_deadline_ns   Not null.
 *   out_has_deadline  Not null.
 *
 *   LOCSIM_OK, *out_has_deadline == 1   *out_deadline_ns is that time.
 *   LOCSIM_OK, *out_has_deadline == 0   There is none: the session is
 *                                       stopped, paused or has failed, or the
 *                                       next sample's time cannot be
 *                                       represented (the next poll reports
 *                                       that). *out_deadline_ns is not written.
 */
LOCSIM_API locsim_status_t locsim_session_next_deadline(
    locsim_session_t session, int64_t *out_deadline_ns, uint8_t *out_has_deadline);

/*
 * Lets the session act on the time without asking for a sample, and
 * reports its state afterwards: a restart attempt that has come due is
 * made, and a running session is checked for a stall. Nothing in the
 * library calls this by itself: a stall is noticed only when the session is
 * called.
 *
 *   out_health  Not null; struct_size set.
 */
LOCSIM_API locsim_status_t locsim_session_check(
    locsim_session_t session, int64_t now_ns, locsim_health_t *out_health);

/* Reports the session's state as of the latest time passed to it. Changes
 * nothing. out_health: not null; struct_size set. */
LOCSIM_API locsim_status_t locsim_session_health(
    locsim_session_t session, locsim_health_t *out_health);

/* Notes that a component other than the session is at fault. The fault
 * stands until it is cleared; while a run exists it makes the health
 * degraded and does nothing else. component is a LOCSIM_COMPONENT_* value,
 * else LOCSIM_ERR_INVALID_ARGUMENT. There is no text. */
LOCSIM_API locsim_status_t locsim_session_report_fault(
    locsim_session_t session, int64_t now_ns, uint32_t component);

/* Notes that a reported fault is over. out_was_set: not null; receives 1
 * if there was one, and 0 if there was none, in which case nothing
 * changed. component as for locsim_session_report_fault. */
LOCSIM_API locsim_status_t locsim_session_clear_fault(
    locsim_session_t session, int64_t now_ns, uint32_t component, uint8_t *out_was_set);

/*
 * Takes the oldest event out of the session's queue.
 *
 *   out_event      Not null; struct_size set.
 *   out_has_event  Not null.
 *
 *   LOCSIM_OK, *out_has_event == 1   *out_event is that event.
 *   LOCSIM_OK, *out_has_event == 0   The queue is empty. *out_event is not
 *                                    written.
 *
 * The queue holds event_capacity events. When one arrives and the queue is
 * full, the oldest is discarded and counted. The totals of locsim_health_t
 * do not depend on the queue. The events of a session that returned
 * LOCSIM_ERR_PANIC are lost.
 */
LOCSIM_API locsim_status_t locsim_session_next_event(
    locsim_session_t session, locsim_event_t *out_event, uint8_t *out_has_event);

/* Says how many events the session has discarded since it was created
 * because its queue was full. The count never goes down. out_count: not
 * null. */
LOCSIM_API locsim_status_t locsim_session_events_dropped(
    locsim_session_t session, uint64_t *out_count);

#ifdef __cplusplus
}
#endif

#endif /* LOCSIM_H */
