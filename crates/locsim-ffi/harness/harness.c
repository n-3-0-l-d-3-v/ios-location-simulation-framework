/*
 * A C caller of locsim-ffi. It includes the public header, links the built
 * library, and checks what the header says against what the library does.
 *
 *   harness <directory of example scenarios> <file to write the samples to>
 *
 * Exit status 0 if every check held, 1 otherwise. Every failed check is
 * printed with its line. The samples file is compared, by the script that
 * runs this, with what the Rust supervisor produces without the C interface.
 *
 * Threads use the Win32 API: that is the only platform this has been built
 * and run on.
 */
#define _CRT_SECURE_NO_WARNINGS
#include "locsim.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef _WIN32
#error "The harness's threads are written for Win32 only; nothing else has been built or run."
#endif
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#define SECOND INT64_C(1000000000)
#define MS INT64_C(1000000)

/* What outputs hold before a call. Still there afterwards: not written. */
#define BYTE_BEFORE 0xA5
#define SESSION_BEFORE UINT64_C(0xDEADBEEFDEADBEEF)
#define I64_BEFORE INT64_C(0x5A5A5A5A5A5A5A5A)
#define U64_BEFORE UINT64_C(0x5A5A5A5A5A5A5A5A)
#define LEN_BEFORE ((size_t)0x5A5A5A5A)

static volatile LONG g_checks;
static volatile LONG g_failures;

#define CHECK(condition)                                                      \
  do {                                                                        \
    InterlockedIncrement(&g_checks);                                          \
    if (!(condition)) {                                                       \
      InterlockedIncrement(&g_failures);                                      \
      fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #condition);    \
    }                                                                         \
  } while (0)

#define CHECK_EQ(actual, expected)                                            \
  do {                                                                        \
    long long actual_ = (long long)(actual);                                  \
    long long expected_ = (long long)(expected);                              \
    InterlockedIncrement(&g_checks);                                          \
    if (actual_ != expected_) {                                               \
      InterlockedIncrement(&g_failures);                                      \
      fprintf(stderr, "FAIL %s:%d: %s is %lld, expected %s = %lld\n",         \
              __FILE__, __LINE__, #actual, actual_, #expected, expected_);    \
    }                                                                         \
  } while (0)

/* ---- Scenario documents -------------------------------------------------- */

#define EXAMPLE_COUNT 8
static const char *const EXAMPLE_NAMES[EXAMPLE_COUNT] = {
    "antimeridian_loop", "circular",    "driving",      "fixed",
    "high_latitude",     "random_walk", "route_replay", "walking"};
#define FIXED 3
#define DRIVING 2

typedef struct {
  uint8_t *bytes;
  size_t len;
} document_t;

static document_t g_examples[EXAMPLE_COUNT];

static int load_examples(const char *directory) {
  for (int i = 0; i < EXAMPLE_COUNT; ++i) {
    char path[1024];
    snprintf(path, sizeof path, "%s/%s.json", directory, EXAMPLE_NAMES[i]);
    FILE *file = fopen(path, "rb");
    if (file == NULL) {
      fprintf(stderr, "cannot open %s\n", path);
      return 0;
    }
    fseek(file, 0, SEEK_END);
    long size = ftell(file);
    fseek(file, 0, SEEK_SET);
    g_examples[i].bytes = (uint8_t *)malloc((size_t)size);
    g_examples[i].len = fread(g_examples[i].bytes, 1, (size_t)size, file);
    fclose(file);
    if (g_examples[i].len != (size_t)size || size == 0) {
      fprintf(stderr, "cannot read %s\n", path);
      return 0;
    }
  }
  return 1;
}

/* ---- Helpers ------------------------------------------------------------- */

static locsim_policy_t good_policy(void) {
  locsim_policy_t policy;
  policy.struct_size = (uint32_t)sizeof policy;
  policy.max_recovery_attempts = 3;
  policy.backoff_initial_s = 1.0;
  policy.backoff_multiplier = 2.0;
  policy.backoff_max_s = 60.0;
  policy.stable_after_s = 10.0;
  policy.stall_after_s = 5.0;
  policy.missed_window_ticks = 10;
  policy.max_missed_in_window = 2;
  return policy;
}

/* A session for one of the examples, which must be accepted. */
static locsim_session_t open_session(int example, uint32_t event_capacity) {
  locsim_policy_t policy = good_policy();
  locsim_session_t session = SESSION_BEFORE;
  size_t err_len = LEN_BEFORE;
  locsim_status_t status = locsim_session_create(
      g_examples[example].bytes, g_examples[example].len, &policy,
      event_capacity, &session, NULL, 0, &err_len);
  CHECK_EQ(status, LOCSIM_OK);
  CHECK(session != 0 && session != SESSION_BEFORE);
  CHECK_EQ(err_len, 0);
  return session;
}

/* Fills a struct with the marker and sets its struct_size. */
static void mark(void *room, size_t size, uint32_t struct_size) {
  memset(room, BYTE_BEFORE, size);
  memcpy(room, &struct_size, sizeof struct_size);
}

/* Whether everything after struct_size is still the marker. */
static int unwritten(const void *room, size_t size, uint32_t struct_size) {
  const uint8_t *bytes = (const uint8_t *)room;
  if (memcmp(room, &struct_size, sizeof struct_size) != 0) return 0;
  for (size_t i = sizeof struct_size; i < size; ++i) {
    if (bytes[i] != BYTE_BEFORE) return 0;
  }
  return 1;
}

static int all_marker(const uint8_t *bytes, size_t size) {
  for (size_t i = 0; i < size; ++i) {
    if (bytes[i] != BYTE_BEFORE) return 0;
  }
  return 1;
}

static uint64_t bits(double value) {
  uint64_t out;
  memcpy(&out, &value, sizeof out);
  return out;
}

typedef struct {
  locsim_status_t status;
  uint8_t has;
  locsim_sample_t sample;
} polled_t;

static polled_t poll_sized(locsim_session_t session, int64_t now, uint32_t struct_size) {
  polled_t out;
  out.has = BYTE_BEFORE;
  mark(&out.sample, sizeof out.sample, struct_size);
  out.status = locsim_session_poll(session, now, &out.sample, &out.has);
  return out;
}

static polled_t poll_at(locsim_session_t session, int64_t now) {
  return poll_sized(session, now, (uint32_t)sizeof(locsim_sample_t));
}

/* A poll that must have produced a sample. */
static locsim_sample_t sample_at(locsim_session_t session, int64_t now) {
  polled_t polled = poll_at(session, now);
  CHECK_EQ(polled.status, LOCSIM_OK);
  CHECK_EQ(polled.has, 1);
  return polled.sample;
}

/* A poll that must have produced nothing and written nothing but the flag. */
static void nothing_at(locsim_session_t session, int64_t now) {
  polled_t polled = poll_at(session, now);
  CHECK_EQ(polled.status, LOCSIM_OK);
  CHECK_EQ(polled.has, 0);
  CHECK(unwritten(&polled.sample, sizeof polled.sample, sizeof polled.sample));
}

typedef struct {
  locsim_status_t status;
  uint8_t has;
  int64_t at;
} deadline_t;

static deadline_t deadline_of(locsim_session_t session) {
  deadline_t out;
  out.has = BYTE_BEFORE;
  out.at = I64_BEFORE;
  out.status = locsim_session_next_deadline(session, &out.at, &out.has);
  return out;
}

static void check_deadline(locsim_session_t session, int has, int64_t at) {
  deadline_t deadline = deadline_of(session);
  CHECK_EQ(deadline.status, LOCSIM_OK);
  CHECK_EQ(deadline.has, has);
  CHECK_EQ(deadline.at, has ? at : I64_BEFORE);
}

static locsim_health_t health_of(locsim_session_t session) {
  locsim_health_t health;
  mark(&health, sizeof health, (uint32_t)sizeof health);
  CHECK_EQ(locsim_session_health(session, &health), LOCSIM_OK);
  CHECK_EQ(health.struct_size, sizeof health);
  CHECK(memcmp(health.reserved, "\0\0\0\0", 4) == 0);
  return health;
}

static locsim_health_t check_at(locsim_session_t session, int64_t now) {
  locsim_health_t health;
  mark(&health, sizeof health, (uint32_t)sizeof health);
  CHECK_EQ(locsim_session_check(session, now, &health), LOCSIM_OK);
  return health;
}

typedef struct {
  locsim_status_t status;
  uint8_t has;
  locsim_event_t event;
} taken_t;

static taken_t take_sized(locsim_session_t session, uint32_t struct_size) {
  taken_t out;
  out.has = BYTE_BEFORE;
  mark(&out.event, sizeof out.event, struct_size);
  out.status = locsim_session_next_event(session, &out.event, &out.has);
  return out;
}

static taken_t take(locsim_session_t session) {
  return take_sized(session, (uint32_t)sizeof(locsim_event_t));
}

/* Empties the queue; returns how many events were waiting. */
static int drain(locsim_session_t session) {
  int count = 0;
  for (;;) {
    taken_t taken = take(session);
    if (taken.status != LOCSIM_OK || taken.has != 1) return count;
    ++count;
  }
}

static uint64_t dropped_of(locsim_session_t session) {
  uint64_t count = U64_BEFORE;
  CHECK_EQ(locsim_session_events_dropped(session, &count), LOCSIM_OK);
  return count;
}

/* ---- 1. Version ---------------------------------------------------------- */

static void test_version(void) {
  CHECK_EQ(locsim_abi_version(), LOCSIM_ABI_VERSION);
  CHECK_EQ(locsim_abi_version(), 1);
}

/* ---- 2. Creation and destruction ----------------------------------------- */

typedef struct {
  locsim_status_t status;
  locsim_session_t session;
  size_t err_len;
  uint8_t buf[8192];
} created_t;

static created_t create_raw(const uint8_t *json, size_t len,
                            const locsim_policy_t *policy, uint32_t capacity,
                            size_t err_cap) {
  created_t out;
  out.session = SESSION_BEFORE;
  out.err_len = LEN_BEFORE;
  memset(out.buf, BYTE_BEFORE, sizeof out.buf);
  out.status = locsim_session_create(json, len, policy, capacity, &out.session,
                                     err_cap ? out.buf : NULL, err_cap, &out.err_len);
  return out;
}

static void test_create_and_destroy(void) {
  const document_t fixed = g_examples[FIXED];
  locsim_policy_t policy = good_policy();

  /* Every example is accepted, and a session is destroyed exactly once. */
  for (int i = 0; i < EXAMPLE_COUNT; ++i) {
    created_t made = create_raw(g_examples[i].bytes, g_examples[i].len, &policy, 16, 8192);
    CHECK_EQ(made.status, LOCSIM_OK);
    CHECK(made.session != 0 && made.session != SESSION_BEFORE);
    CHECK_EQ(made.err_len, 0);
    CHECK(all_marker(made.buf, sizeof made.buf));
    CHECK_EQ(locsim_session_destroy(made.session), LOCSIM_OK);
    CHECK_EQ(locsim_session_destroy(made.session), LOCSIM_ERR_INVALID_HANDLE);
  }

  /* Handles that name nothing. */
  CHECK_EQ(locsim_session_destroy(0), LOCSIM_ERR_INVALID_HANDLE);
  CHECK_EQ(locsim_session_destroy(UINT64_MAX), LOCSIM_ERR_INVALID_HANDLE);
  CHECK_EQ(locsim_session_destroy(UINT64_C(1) << 32), LOCSIM_ERR_INVALID_HANDLE);

  /* A stale handle does not reach the session created after it. */
  {
    locsim_session_t old = open_session(FIXED, 16);
    CHECK_EQ(locsim_session_destroy(old), LOCSIM_OK);
    locsim_session_t new_one = open_session(FIXED, 16);
    CHECK(new_one != old);
    CHECK_EQ(locsim_session_start(old, 0), LOCSIM_ERR_INVALID_HANDLE);
    CHECK_EQ(health_of(new_one).lifecycle, LOCSIM_STATE_IDLE);
    CHECK_EQ(locsim_session_destroy(new_one), LOCSIM_OK);
  }

  /* A null pointer: refused, and nothing at all is written. */
  {
    locsim_session_t session = SESSION_BEFORE;
    size_t err_len = LEN_BEFORE;
    uint8_t buf[32];
    memset(buf, BYTE_BEFORE, sizeof buf);
#define NULL_CASE(call)                                   \
  CHECK_EQ(call, LOCSIM_ERR_NULL_POINTER);                \
  CHECK(session == SESSION_BEFORE);                       \
  CHECK(err_len == LEN_BEFORE);                           \
  CHECK(all_marker(buf, sizeof buf))
    NULL_CASE(locsim_session_create(NULL, fixed.len, &policy, 16, &session, buf, sizeof buf, &err_len));
    NULL_CASE(locsim_session_create(fixed.bytes, fixed.len, NULL, 16, &session, buf, sizeof buf, &err_len));
    NULL_CASE(locsim_session_create(fixed.bytes, fixed.len, &policy, 16, NULL, buf, sizeof buf, &err_len));
    NULL_CASE(locsim_session_create(fixed.bytes, fixed.len, &policy, 16, &session, NULL, sizeof buf, &err_len));
    NULL_CASE(locsim_session_create(fixed.bytes, fixed.len, &policy, 16, &session, buf, sizeof buf, NULL));
#undef NULL_CASE
  }

  /* A null document of length 0 is an empty document, not a null pointer. */
  {
    created_t made = create_raw(NULL, 0, &policy, 16, 0);
    CHECK_EQ(made.status, LOCSIM_ERR_SCENARIO);
    CHECK_EQ(made.session, 0);
    CHECK(made.err_len > 0 && made.err_len <= LOCSIM_CREATE_ERROR_TEXT_MAX);
  }

  /* Refusals without a text: the handle is zeroed and the length is 0. */
  {
    locsim_policy_t wrong_size = policy;
    wrong_size.struct_size = 63;
    const uint8_t not_utf8[3] = {'{', 0xFF, '}'};
    created_t cases[5];
    cases[0] = create_raw(fixed.bytes, fixed.len, &wrong_size, 16, 8192);
    cases[1] = create_raw(fixed.bytes, fixed.len, &policy, 0, 8192);
    cases[2] = create_raw(fixed.bytes, fixed.len, &policy, LOCSIM_EVENT_CAPACITY_MAX + 1, 8192);
    cases[3] = create_raw(not_utf8, sizeof not_utf8, &policy, 16, 8192);
    /* Several faults at once: the struct size is reported first. */
    cases[4] = create_raw(not_utf8, sizeof not_utf8, &wrong_size, 0, 8192);
    const locsim_status_t expected[5] = {
        LOCSIM_ERR_STRUCT_SIZE, LOCSIM_ERR_INVALID_ARGUMENT, LOCSIM_ERR_INVALID_ARGUMENT,
        LOCSIM_ERR_INVALID_UTF8, LOCSIM_ERR_STRUCT_SIZE};
    for (int i = 0; i < 5; ++i) {
      CHECK_EQ(cases[i].status, expected[i]);
      CHECK_EQ(cases[i].session, 0);
      CHECK_EQ(cases[i].err_len, 0);
      CHECK(all_marker(cases[i].buf, sizeof cases[i].buf));
    }
    /* The capacity's limits are themselves accepted. */
    created_t least = create_raw(fixed.bytes, fixed.len, &policy, 1, 0);
    created_t most = create_raw(fixed.bytes, fixed.len, &policy, LOCSIM_EVENT_CAPACITY_MAX, 0);
    CHECK_EQ(least.status, LOCSIM_OK);
    CHECK_EQ(most.status, LOCSIM_OK);
    CHECK_EQ(locsim_session_destroy(least.session), LOCSIM_OK);
    CHECK_EQ(locsim_session_destroy(most.session), LOCSIM_OK);
  }

  /* A refused scenario: the text is written whole or not at all, and its
   * length is reported either way. */
  {
    const uint8_t broken[1] = {'{'};
    created_t whole = create_raw(broken, 1, &policy, 16, 8192);
    CHECK_EQ(whole.status, LOCSIM_ERR_SCENARIO);
    CHECK_EQ(whole.session, 0);
    size_t len = whole.err_len;
    CHECK(len > 1 && len < 4096);
    CHECK(!all_marker(whole.buf, len));
    CHECK(all_marker(whole.buf + len, sizeof whole.buf - len));

    created_t none = create_raw(broken, 1, &policy, 16, 0);
    CHECK_EQ(none.status, LOCSIM_ERR_SCENARIO);
    CHECK_EQ(none.err_len, len);

    created_t one_short = create_raw(broken, 1, &policy, 16, len - 1);
    CHECK_EQ(one_short.status, LOCSIM_ERR_SCENARIO);
    CHECK_EQ(one_short.err_len, len);
    CHECK_EQ(one_short.session, 0);
    CHECK(all_marker(one_short.buf, sizeof one_short.buf));

    created_t exact = create_raw(broken, 1, &policy, 16, len);
    CHECK_EQ(exact.status, LOCSIM_ERR_SCENARIO);
    CHECK_EQ(exact.err_len, len);
    CHECK(memcmp(exact.buf, whole.buf, len) == 0);
    CHECK(all_marker(exact.buf + len, sizeof exact.buf - len));
  }

  /* A refused policy names the field. */
  {
    locsim_policy_t bad = policy;
    bad.backoff_multiplier = 0.5;
    created_t made = create_raw(fixed.bytes, fixed.len, &bad, 16, 8192);
    CHECK_EQ(made.status, LOCSIM_ERR_POLICY);
    CHECK_EQ(made.session, 0);
    const char expected[] = "backoff_multiplier: ";
    CHECK(made.err_len > sizeof expected - 1);
    CHECK(memcmp(made.buf, expected, sizeof expected - 1) == 0);
  }
}

/* ---- 3. Lifecycle -------------------------------------------------------- */

static void test_lifecycle(void) {
  locsim_session_t session = open_session(FIXED, 64);
  /* Stopped: only start is allowed. */
  CHECK_EQ(locsim_session_stop(session), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(locsim_session_pause(session, 0), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(locsim_session_resume(session, 0), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(locsim_session_start(session, 0), LOCSIM_OK);
  /* Running. */
  CHECK_EQ(locsim_session_start(session, 0), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(locsim_session_resume(session, 0), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(sample_at(session, 0).timestamp_ns, 0);
  CHECK_EQ(locsim_session_pause(session, 2 * SECOND), LOCSIM_OK);
  /* Paused. */
  CHECK_EQ(health_of(session).lifecycle, LOCSIM_STATE_PAUSED);
  CHECK_EQ(locsim_session_pause(session, 2 * SECOND), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(locsim_session_start(session, 2 * SECOND), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(locsim_session_resume(session, SECOND), LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS);
  CHECK_EQ(locsim_session_resume(session, SECOND), LOCSIM_ERR_SCHEDULE_CLOCK_WENT_BACKWARDS);
  nothing_at(session, 9 * SECOND);
  CHECK_EQ(locsim_session_resume(session, 9 * SECOND), LOCSIM_OK);
  CHECK_EQ(health_of(session).lifecycle, LOCSIM_STATE_RUNNING);
  CHECK_EQ(locsim_session_stop(session), LOCSIM_OK);
  CHECK_EQ(locsim_session_stop(session), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(health_of(session).lifecycle, LOCSIM_STATE_IDLE);
  /* Ten calls were refused (three while stopped, two while running, four
   * while paused, one after stopping); none of them failed a run. */
  CHECK_EQ(health_of(session).total_rejected_operations, 10);
  CHECK_EQ(health_of(session).total_failures, 0);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
}

/* ---- 9. A scenario, sample by sample -------------------------------------- */

static void test_samples(void) {
  /* fixed.json: one sample a second around 12.9352 N, 77.6245 E. */
  locsim_session_t session = open_session(FIXED, 64);
  check_deadline(session, 0, 0);
  nothing_at(session, 1000);

  CHECK_EQ(locsim_session_start(session, 1000), LOCSIM_OK);
  check_deadline(session, 1, 1000);
  locsim_sample_t first = sample_at(session, 1000);
  CHECK_EQ(first.struct_size, sizeof first);
  CHECK_EQ(first.timestamp_ns, 1000);
  CHECK_EQ(first.run, 1);
  CHECK_EQ(first.source, LOCSIM_SOURCE_SIMULATION);
  CHECK_EQ(first.simulation_state, LOCSIM_STATE_RUNNING);
  CHECK(first.reserved[0] == 0 && first.reserved[1] == 0);
  CHECK(first.latitude_deg > 12.93 && first.latitude_deg < 12.94);
  CHECK(first.longitude_deg > 77.62 && first.longitude_deg < 77.63);
  CHECK(first.altitude_m > 800.0 && first.altitude_m < 1000.0);
  CHECK(first.horizontal_accuracy_m > 0.0 && first.vertical_accuracy_m > 0.0);
  /* The first sample of a run has neither speed nor course: not zero. */
  CHECK_EQ(first.has_speed, 0);
  CHECK_EQ(first.has_course, 0);
  CHECK(first.speed_mps != first.speed_mps);
  CHECK(first.course_deg != first.course_deg);

  /* One nanosecond early there is nothing; on the tick there is. */
  check_deadline(session, 1, 1000 + SECOND);
  nothing_at(session, 1000 + SECOND - 1);
  locsim_sample_t second = sample_at(session, 1000 + SECOND);
  CHECK_EQ(second.timestamp_ns, 1000 + SECOND);
  CHECK_EQ(second.has_speed, 1);
  CHECK(second.speed_mps >= 0.0);
  if (second.has_course) CHECK(second.course_deg >= 0.0 && second.course_deg < 360.0);

  /* A late poll gets the latest tick that is due, with that tick's time. */
  locsim_sample_t late = sample_at(session, 1000 + 3 * SECOND + 500 * MS);
  CHECK_EQ(late.timestamp_ns, 1000 + 3 * SECOND);
  check_deadline(session, 1, 1000 + 4 * SECOND);
  CHECK_EQ(health_of(session).current_missed_ticks, 1);

  /* Paused and stopped sessions have no deadline and no sample. */
  CHECK_EQ(locsim_session_pause(session, 1000 + 4 * SECOND), LOCSIM_OK);
  check_deadline(session, 0, 0);
  nothing_at(session, 1000 + 9 * SECOND);
  CHECK_EQ(locsim_session_stop(session), LOCSIM_OK);
  check_deadline(session, 0, 0);

  /* A second start is a second run, and its first sample has no speed. */
  CHECK_EQ(locsim_session_start(session, 50 * SECOND), LOCSIM_OK);
  locsim_sample_t again = sample_at(session, 50 * SECOND);
  CHECK_EQ(again.timestamp_ns, 50 * SECOND);
  CHECK_EQ(again.run, 2);
  CHECK_EQ(again.has_speed, 0);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);

  /* Negative times are times like any other. */
  session = open_session(FIXED, 64);
  CHECK_EQ(locsim_session_start(session, -3 * SECOND), LOCSIM_OK);
  for (int64_t k = -3; k <= 1; ++k) {
    check_deadline(session, 1, k * SECOND);
    CHECK_EQ(sample_at(session, k * SECOND).timestamp_ns, k * SECOND);
  }
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);

  /* A run whose next time cannot be represented fails once, and for good. */
  session = open_session(FIXED, 64);
  const int64_t start = INT64_MAX - SECOND / 2;
  CHECK_EQ(locsim_session_start(session, start), LOCSIM_OK);
  CHECK_EQ(sample_at(session, start).timestamp_ns, start);
  check_deadline(session, 0, 0);
  polled_t failed = poll_at(session, INT64_MAX);
  CHECK_EQ(failed.status, LOCSIM_ERR_SCHEDULE_OVERFLOW);
  CHECK_EQ(failed.has, BYTE_BEFORE);
  CHECK(unwritten(&failed.sample, sizeof failed.sample, sizeof failed.sample));
  nothing_at(session, INT64_MAX);
  locsim_health_t health = health_of(session);
  CHECK_EQ(health.health, LOCSIM_HEALTH_FAILED);
  CHECK_EQ(health.lifecycle, LOCSIM_STATE_ERROR);
  CHECK_EQ(health.last_error, LOCSIM_ERR_SCHEDULE_OVERFLOW);
  CHECK_EQ(health.has_retry_at, 0);
  CHECK_EQ(health.retry_at_ns, 0);
  CHECK_EQ(health.total_terminal_failures, 1);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
}

/* ---- 4. Health ----------------------------------------------------------- */

static void test_health(void) {
  locsim_session_t session = open_session(FIXED, 64);

  /* Never called with a time: every time is absent, flag 0 and value 0. */
  locsim_health_t fresh = health_of(session);
  CHECK_EQ(fresh.lifecycle, LOCSIM_STATE_IDLE);
  CHECK_EQ(fresh.health, LOCSIM_HEALTH_STOPPED);
  CHECK_EQ(fresh.emission, LOCSIM_EMISSION_NOT_EXPECTED);
  CHECK_EQ(fresh.causes, 0);
  CHECK_EQ(fresh.active_faults, 0);
  CHECK_EQ(fresh.attempts_used, 0);
  CHECK_EQ(fresh.max_attempts, 3);
  CHECK_EQ(fresh.last_error, 0);
  CHECK_EQ(fresh.current_state, LOCSIM_STATE_IDLE);
  CHECK_EQ(fresh.has_observed_at, 0);
  CHECK_EQ(fresh.has_retry_at, 0);
  CHECK_EQ(fresh.has_last_sample_time, 0);
  CHECK_EQ(fresh.trajectory_complete, 0);
  CHECK_EQ(fresh.observed_at_ns, 0);
  CHECK_EQ(fresh.retry_at_ns, 0);
  CHECK_EQ(fresh.silent_for_ns, 0);
  CHECK_EQ(fresh.last_sample_time_ns, 0);
  CHECK_EQ(fresh.run, 0);
  CHECK_EQ(fresh.total_runs_started, 0);
  CHECK_EQ(fresh.total_samples, 0);

  /* A check passes the time on; health alone does not. */
  locsim_health_t checked = check_at(session, 5);
  CHECK_EQ(checked.has_observed_at, 1);
  CHECK_EQ(checked.observed_at_ns, 5);
  CHECK_EQ(health_of(session).observed_at_ns, 5);

  CHECK_EQ(locsim_session_start(session, 10), LOCSIM_OK);
  CHECK_EQ(sample_at(session, 10).timestamp_ns, 10);
  locsim_health_t running = health_of(session);
  CHECK_EQ(running.lifecycle, LOCSIM_STATE_RUNNING);
  CHECK_EQ(running.current_state, LOCSIM_STATE_RUNNING);
  CHECK_EQ(running.health, LOCSIM_HEALTH_HEALTHY);
  CHECK_EQ(running.emission, LOCSIM_EMISSION_FLOWING);
  CHECK_EQ(running.run, 1);
  CHECK_EQ(running.has_last_sample_time, 1);
  CHECK_EQ(running.last_sample_time_ns, 10);
  CHECK_EQ(running.current_sample_count, 1);
  CHECK_EQ(running.total_runs_started, 1);
  CHECK_EQ(running.total_samples, 1);
  CHECK_EQ(running.has_retry_at, 0);

  /* Nothing asked for a sample for twenty seconds: a stall, noticed only
   * because check was called. */
  locsim_health_t stalled = check_at(session, 10 + 20 * SECOND);
  CHECK_EQ(stalled.lifecycle, LOCSIM_STATE_RUNNING);
  CHECK_EQ(stalled.health, LOCSIM_HEALTH_DEGRADED);
  CHECK_EQ(stalled.emission, LOCSIM_EMISSION_STALLED);
  CHECK_EQ(stalled.causes, LOCSIM_CAUSE_STALLED);
  CHECK_EQ(stalled.silent_for_ns, 20 * SECOND);

  CHECK_EQ(locsim_session_stop(session), LOCSIM_OK);
  locsim_health_t stopped = health_of(session);
  CHECK_EQ(stopped.health, LOCSIM_HEALTH_STOPPED);
  CHECK_EQ(stopped.emission, LOCSIM_EMISSION_NOT_EXPECTED);
  CHECK_EQ(stopped.total_runs_ended, 1);
  CHECK_EQ(stopped.total_samples, 1);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
}

/* ---- 5. Faults ----------------------------------------------------------- */

static void test_faults(void) {
  const uint32_t components[3] = {LOCSIM_COMPONENT_STORE, LOCSIM_COMPONENT_ADAPTER,
                                  LOCSIM_COMPONENT_DELIVERY};
  locsim_session_t session = open_session(FIXED, 64);

  /* Each component, alone: reported, listed, cleared once. */
  for (int i = 0; i < 3; ++i) {
    uint8_t was_set = BYTE_BEFORE;
    CHECK_EQ(locsim_session_report_fault(session, 100 + i, components[i]), LOCSIM_OK);
    CHECK_EQ(health_of(session).active_faults, 1u << (components[i] - 1));
    /* Stopped: listed, and the health is still "stopped". */
    CHECK_EQ(health_of(session).health, LOCSIM_HEALTH_STOPPED);
    CHECK_EQ(locsim_session_clear_fault(session, 200 + i, components[i], &was_set), LOCSIM_OK);
    CHECK_EQ(was_set, 1);
    CHECK_EQ(health_of(session).active_faults, 0);
    was_set = BYTE_BEFORE;
    CHECK_EQ(locsim_session_clear_fault(session, 300 + i, components[i], &was_set), LOCSIM_OK);
    CHECK_EQ(was_set, 0);
  }

  /* All three at once, with a run: degraded, for that reason alone. */
  CHECK_EQ(locsim_session_start(session, 1000), LOCSIM_OK);
  CHECK_EQ(sample_at(session, 1000).timestamp_ns, 1000);
  for (int i = 0; i < 3; ++i) {
    CHECK_EQ(locsim_session_report_fault(session, 1000, components[i]), LOCSIM_OK);
  }
  locsim_health_t degraded = health_of(session);
  CHECK_EQ(degraded.active_faults, 7);
  CHECK_EQ(degraded.health, LOCSIM_HEALTH_DEGRADED);
  CHECK_EQ(degraded.causes, LOCSIM_CAUSE_EXTERNAL_FAULT);
  CHECK_EQ(degraded.emission, LOCSIM_EMISSION_FLOWING);
  for (int i = 0; i < 3; ++i) {
    uint8_t was_set = BYTE_BEFORE;
    CHECK_EQ(locsim_session_clear_fault(session, 1000, components[i], &was_set), LOCSIM_OK);
    CHECK_EQ(was_set, 1);
  }
  CHECK_EQ(health_of(session).health, LOCSIM_HEALTH_HEALTHY);

  /* A component that does not exist, before the handle is looked at. */
  const uint32_t unknown[3] = {0, 4, UINT32_MAX};
  for (int i = 0; i < 3; ++i) {
    uint8_t was_set = BYTE_BEFORE;
    CHECK_EQ(locsim_session_report_fault(session, 0, unknown[i]), LOCSIM_ERR_INVALID_ARGUMENT);
    CHECK_EQ(locsim_session_report_fault(0, 0, unknown[i]), LOCSIM_ERR_INVALID_ARGUMENT);
    CHECK_EQ(locsim_session_clear_fault(session, 0, unknown[i], &was_set), LOCSIM_ERR_INVALID_ARGUMENT);
    CHECK_EQ(was_set, BYTE_BEFORE);
  }
  CHECK_EQ(locsim_session_clear_fault(session, 0, 99, NULL), LOCSIM_ERR_NULL_POINTER);
  CHECK_EQ(health_of(session).active_faults, 0);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
}

/* ---- 6. Events ----------------------------------------------------------- */

static void test_events(void) {
  /* An empty queue: the flag is 0 and the event is not written. */
  locsim_session_t session = open_session(FIXED, 64);
  taken_t empty = take(session);
  CHECK_EQ(empty.status, LOCSIM_OK);
  CHECK_EQ(empty.has, 0);
  CHECK(unwritten(&empty.event, sizeof empty.event, sizeof empty.event));
  CHECK_EQ(dropped_of(session), 0);

  /* Order: a refused stop, then a start, which is a run and a change of
   * health from stopped to healthy. */
  CHECK_EQ(locsim_session_stop(session), LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(locsim_session_start(session, 77), LOCSIM_OK);
  taken_t refused = take(session);
  CHECK_EQ(refused.has, 1);
  CHECK_EQ(refused.event.struct_size, sizeof refused.event);
  CHECK_EQ(refused.event.kind, LOCSIM_EVENT_OPERATION_REJECTED);
  CHECK_EQ(refused.event.level, LOCSIM_LEVEL_WARN);
  CHECK_EQ(refused.event.error, LOCSIM_ERR_PROVIDER_TRANSITION);
  CHECK_EQ(refused.event.arg[0], 1);
  CHECK_EQ(refused.event.run, 0);
  taken_t started = take(session);
  CHECK_EQ(started.event.kind, LOCSIM_EVENT_RUN_STARTED);
  CHECK_EQ(started.event.level, LOCSIM_LEVEL_INFO);
  CHECK_EQ(started.event.at_ns, 77);
  CHECK_EQ(started.event.run, 1);
  CHECK_EQ(started.event.arg[0], 0);
  CHECK_EQ(started.event.error, 0);
  taken_t changed = take(session);
  CHECK_EQ(changed.event.kind, LOCSIM_EVENT_HEALTH_CHANGED);
  CHECK_EQ(changed.event.arg[0], LOCSIM_HEALTH_STOPPED);
  CHECK_EQ(changed.event.arg[1], LOCSIM_HEALTH_HEALTHY);
  CHECK_EQ(take(session).has, 0);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);

  /* A queue of three, given five events: the newest three, in order, and
   * two counted as dropped. */
  session = open_session(FIXED, 3);
  for (int64_t t = 1; t <= 5; ++t) {
    CHECK_EQ(locsim_session_report_fault(session, t, LOCSIM_COMPONENT_ADAPTER), LOCSIM_OK);
  }
  CHECK_EQ(dropped_of(session), 2);
  for (int64_t t = 3; t <= 5; ++t) {
    taken_t taken = take(session);
    CHECK_EQ(taken.status, LOCSIM_OK);
    CHECK_EQ(taken.has, 1);
    CHECK_EQ(taken.event.kind, LOCSIM_EVENT_FAULT_REPORTED);
    CHECK_EQ(taken.event.at_ns, t);
    CHECK_EQ(taken.event.arg[0], LOCSIM_COMPONENT_ADAPTER);
    CHECK_EQ(taken.event.arg[1], 0);
    CHECK_EQ(taken.event.level, LOCSIM_LEVEL_WARN);
  }
  CHECK_EQ(take(session).has, 0);
  /* Emptying the queue does not forget what was dropped, and room made by
   * taking events out is room for new ones. */
  CHECK_EQ(dropped_of(session), 2);
  for (int64_t t = 6; t <= 8; ++t) {
    CHECK_EQ(locsim_session_report_fault(session, t, LOCSIM_COMPONENT_STORE), LOCSIM_OK);
  }
  CHECK_EQ(dropped_of(session), 2);
  CHECK_EQ(drain(session), 3);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
}

/* ---- 7 and 8. Refused calls write nothing --------------------------------- */

static void refused_everywhere(locsim_session_t session, locsim_status_t status) {
  CHECK_EQ(locsim_session_start(session, 0), status);
  CHECK_EQ(locsim_session_stop(session), status);
  CHECK_EQ(locsim_session_pause(session, 0), status);
  CHECK_EQ(locsim_session_resume(session, 0), status);
  CHECK_EQ(locsim_session_report_fault(session, 0, LOCSIM_COMPONENT_STORE), status);

  polled_t polled = poll_at(session, 0);
  CHECK_EQ(polled.status, status);
  CHECK_EQ(polled.has, BYTE_BEFORE);
  CHECK(unwritten(&polled.sample, sizeof polled.sample, sizeof polled.sample));

  deadline_t deadline = deadline_of(session);
  CHECK_EQ(deadline.status, status);
  CHECK_EQ(deadline.has, BYTE_BEFORE);
  CHECK_EQ(deadline.at, I64_BEFORE);

  locsim_health_t health;
  mark(&health, sizeof health, (uint32_t)sizeof health);
  CHECK_EQ(locsim_session_health(session, &health), status);
  CHECK(unwritten(&health, sizeof health, sizeof health));
  CHECK_EQ(locsim_session_check(session, 0, &health), status);
  CHECK(unwritten(&health, sizeof health, sizeof health));

  uint8_t was_set = BYTE_BEFORE;
  CHECK_EQ(locsim_session_clear_fault(session, 0, LOCSIM_COMPONENT_STORE, &was_set), status);
  CHECK_EQ(was_set, BYTE_BEFORE);

  taken_t taken = take(session);
  CHECK_EQ(taken.status, status);
  CHECK_EQ(taken.has, BYTE_BEFORE);
  CHECK(unwritten(&taken.event, sizeof taken.event, sizeof taken.event));

  uint64_t count = U64_BEFORE;
  CHECK_EQ(locsim_session_events_dropped(session, &count), status);
  CHECK(count == U64_BEFORE);
}

static void test_refusals(void) {
  /* Handles that name no session. */
  locsim_session_t destroyed = open_session(FIXED, 16);
  CHECK_EQ(locsim_session_start(destroyed, 0), LOCSIM_OK);
  CHECK_EQ(locsim_session_destroy(destroyed), LOCSIM_OK);
  refused_everywhere(0, LOCSIM_ERR_INVALID_HANDLE);
  refused_everywhere(UINT64_MAX, LOCSIM_ERR_INVALID_HANDLE);
  refused_everywhere(destroyed, LOCSIM_ERR_INVALID_HANDLE);

  locsim_session_t session = open_session(FIXED, 16);
  CHECK_EQ(locsim_session_start(session, 0), LOCSIM_OK);

  /* A wrong struct_size, with a good handle and with none. */
  const locsim_session_t handles[2] = {session, 0};
  const uint32_t sample_sizes[3] = {0, 87, 89};
  const uint32_t health_sizes[3] = {0, 191, 193};
  const uint32_t event_sizes[3] = {0, 63, 65};
  for (int h = 0; h < 2; ++h) {
    for (int i = 0; i < 3; ++i) {
      polled_t polled = poll_sized(handles[h], 0, sample_sizes[i]);
      CHECK_EQ(polled.status, LOCSIM_ERR_STRUCT_SIZE);
      CHECK_EQ(polled.has, BYTE_BEFORE);
      CHECK(unwritten(&polled.sample, sizeof polled.sample, sample_sizes[i]));

      locsim_health_t health;
      mark(&health, sizeof health, health_sizes[i]);
      CHECK_EQ(locsim_session_health(handles[h], &health), LOCSIM_ERR_STRUCT_SIZE);
      CHECK(unwritten(&health, sizeof health, health_sizes[i]));
      CHECK_EQ(locsim_session_check(handles[h], 5, &health), LOCSIM_ERR_STRUCT_SIZE);
      CHECK(unwritten(&health, sizeof health, health_sizes[i]));

      taken_t taken = take_sized(handles[h], event_sizes[i]);
      CHECK_EQ(taken.status, LOCSIM_ERR_STRUCT_SIZE);
      CHECK_EQ(taken.has, BYTE_BEFORE);
      CHECK(unwritten(&taken.event, sizeof taken.event, event_sizes[i]));
    }

    /* A null pointer, which comes before everything else; what is beside
     * it is not written. */
    uint8_t has = BYTE_BEFORE;
    int64_t at = I64_BEFORE;
    locsim_sample_t sample;
    locsim_event_t event;
    mark(&sample, sizeof sample, 1);
    mark(&event, sizeof event, 1);
    CHECK_EQ(locsim_session_poll(handles[h], 0, NULL, &has), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_poll(handles[h], 0, &sample, NULL), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_next_deadline(handles[h], NULL, &has), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_next_deadline(handles[h], &at, NULL), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_health(handles[h], NULL), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_check(handles[h], 0, NULL), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_next_event(handles[h], NULL, &has), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_next_event(handles[h], &event, NULL), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(locsim_session_events_dropped(handles[h], NULL), LOCSIM_ERR_NULL_POINTER);
    CHECK_EQ(has, BYTE_BEFORE);
    CHECK_EQ(at, I64_BEFORE);
    CHECK(unwritten(&sample, sizeof sample, 1));
    CHECK(unwritten(&event, sizeof event, 1));
  }

  /* None of the refused calls reached the session: its first sample and
   * its first events are still waiting, and it has seen no time but 0. */
  CHECK_EQ(health_of(session).observed_at_ns, 0);
  CHECK_EQ(sample_at(session, 0).timestamp_ns, 0);
  CHECK_EQ(drain(session), 2);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
}

/* ---- 9, continued. Every example, written out for comparison -------------- */

#define DUMP_SAMPLES 60

/* Drives a session by its own deadlines and hashes or writes every sample. */
static uint64_t run_by_deadline(int example, FILE *out) {
  uint64_t hash = UINT64_C(0xcbf29ce484222325);
  locsim_session_t session = open_session(example, 64);
  CHECK_EQ(locsim_session_start(session, 0), LOCSIM_OK);
  for (int n = 0; n < DUMP_SAMPLES; ++n) {
    deadline_t deadline = deadline_of(session);
    CHECK_EQ(deadline.status, LOCSIM_OK);
    CHECK_EQ(deadline.has, 1);
    locsim_sample_t sample = sample_at(session, deadline.at);
    CHECK_EQ(sample.timestamp_ns, deadline.at);
    const uint8_t *bytes = (const uint8_t *)&sample;
    for (size_t i = 0; i < sizeof sample; ++i) {
      hash = (hash ^ bytes[i]) * UINT64_C(0x100000001b3);
    }
    if (out != NULL) {
      fprintf(out,
              "%s %" PRId64 " %" PRIu64 " %u %u %u %u %016" PRIx64 " %016" PRIx64
              " %016" PRIx64 " %016" PRIx64 " %016" PRIx64 " %016" PRIx64 " %016" PRIx64 "\n",
              EXAMPLE_NAMES[example], sample.timestamp_ns, sample.run,
              (unsigned)sample.source, (unsigned)sample.simulation_state,
              (unsigned)sample.has_speed, (unsigned)sample.has_course,
              bits(sample.latitude_deg), bits(sample.longitude_deg),
              bits(sample.altitude_m), bits(sample.horizontal_accuracy_m),
              bits(sample.vertical_accuracy_m),
              sample.has_speed ? bits(sample.speed_mps) : 0,
              sample.has_course ? bits(sample.course_deg) : 0);
    }
  }
  locsim_health_t health = health_of(session);
  CHECK_EQ(health.total_samples, DUMP_SAMPLES);
  CHECK_EQ(health.current_missed_ticks, 0);
  if (out != NULL) {
    fprintf(out, "%s totals %" PRIu64 " %" PRIu64 " %" PRIu64 "\n", EXAMPLE_NAMES[example],
            health.total_runs_started, health.total_samples, health.total_missed_ticks);
  }
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
  return hash;
}

static void test_dump(const char *path) {
  FILE *out = fopen(path, "wb");
  CHECK(out != NULL);
  if (out == NULL) return;
  for (int i = 0; i < EXAMPLE_COUNT; ++i) {
    uint64_t written = run_by_deadline(i, out);
    /* The same scenario again gives the same samples, byte for byte. */
    CHECK(run_by_deadline(i, NULL) == written);
  }
  fclose(out);
}

/* ---- 10. Create, use, destroy, many times --------------------------------- */

static void test_cycles(void) {
  locsim_session_t previous = 0;
  for (int n = 0; n < 3000; ++n) {
    locsim_session_t session = open_session(n % EXAMPLE_COUNT, 8);
    CHECK(session != previous);
    CHECK_EQ(locsim_session_start(session, n), LOCSIM_OK);
    CHECK_EQ(sample_at(session, n).timestamp_ns, n);
    CHECK_EQ(health_of(session).total_samples, 1);
    if (previous != 0) {
      CHECK_EQ(locsim_session_stop(previous), LOCSIM_ERR_INVALID_HANDLE);
    }
    CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
    previous = session;
  }

  /* As many sessions as the limit, and not one more. */
  static locsim_session_t sessions[LOCSIM_SESSION_LIMIT];
  locsim_policy_t policy = good_policy();
  for (unsigned n = 0; n < LOCSIM_SESSION_LIMIT; ++n) {
    sessions[n] = open_session(FIXED, 1);
  }
  created_t over = create_raw(g_examples[FIXED].bytes, g_examples[FIXED].len, &policy, 1, 0);
  CHECK_EQ(over.status, LOCSIM_ERR_LIMIT);
  CHECK_EQ(over.session, 0);
  CHECK_EQ(over.err_len, 0);
  for (unsigned n = 0; n < LOCSIM_SESSION_LIMIT; ++n) {
    CHECK_EQ(locsim_session_destroy(sessions[n]), LOCSIM_OK);
  }
  locsim_session_t after = open_session(FIXED, 1);
  CHECK_EQ(locsim_session_destroy(after), LOCSIM_OK);
}

/* ---- Threads ------------------------------------------------------------- */

/* Threads announce that they are ready and then wait to be let go together. */
typedef struct {
  HANDLE ready;
  HANDLE go;
} gate_t;

static gate_t gate_new(void) {
  gate_t gate;
  gate.ready = CreateSemaphoreA(NULL, 0, 64, NULL);
  gate.go = CreateEventA(NULL, TRUE, FALSE, NULL);
  return gate;
}

static void gate_wait(const gate_t *gate) {
  ReleaseSemaphore(gate->ready, 1, NULL);
  WaitForSingleObject(gate->go, INFINITE);
}

static void gate_open(const gate_t *gate, int threads) {
  for (int i = 0; i < threads; ++i) WaitForSingleObject(gate->ready, INFINITE);
  SetEvent(gate->go);
}

static void gate_free(gate_t *gate) {
  CloseHandle(gate->ready);
  CloseHandle(gate->go);
}

/* Two threads on one session. */
#define SHARED_POLLS 40000
typedef struct {
  const gate_t *gate;
  locsim_session_t session;
  int reads_too; /* also ask for health, deadlines and events */
  long ok, busy, other, samples;
} sharer_t;

static DWORD WINAPI share_session(LPVOID argument) {
  sharer_t *me = (sharer_t *)argument;
  gate_wait(me->gate);
  for (int k = 0; k < SHARED_POLLS; ++k) {
    polled_t polled = poll_at(me->session, (int64_t)k * (SECOND / 4));
    if (polled.status == LOCSIM_OK) {
      ++me->ok;
      CHECK(polled.has == 0 || polled.has == 1);
      if (polled.has == 1) {
        ++me->samples;
        CHECK_EQ(polled.sample.struct_size, sizeof polled.sample);
        CHECK_EQ(polled.sample.timestamp_ns % SECOND, 0);
      } else {
        CHECK(unwritten(&polled.sample, sizeof polled.sample, sizeof polled.sample));
      }
    } else if (polled.status == LOCSIM_ERR_BUSY) {
      /* Refused at once, and nothing was written. */
      ++me->busy;
      CHECK_EQ(polled.has, BYTE_BEFORE);
      CHECK(unwritten(&polled.sample, sizeof polled.sample, sizeof polled.sample));
    } else {
      ++me->other;
    }
    if (me->reads_too) {
      locsim_health_t health;
      mark(&health, sizeof health, (uint32_t)sizeof health);
      locsim_status_t status = locsim_session_health(me->session, &health);
      if (status == LOCSIM_ERR_BUSY) {
        ++me->busy;
        CHECK(unwritten(&health, sizeof health, sizeof health));
      } else if (status == LOCSIM_OK) {
        CHECK_EQ(health.lifecycle, LOCSIM_STATE_RUNNING);
      } else {
        ++me->other;
      }
      deadline_t deadline = deadline_of(me->session);
      if (deadline.status == LOCSIM_ERR_BUSY) {
        ++me->busy;
        CHECK(deadline.has == BYTE_BEFORE && deadline.at == I64_BEFORE);
      } else if (deadline.status != LOCSIM_OK) {
        ++me->other;
      }
      taken_t taken = take(me->session);
      if (taken.status == LOCSIM_ERR_BUSY) {
        ++me->busy;
        CHECK(taken.has == BYTE_BEFORE);
      } else if (taken.status != LOCSIM_OK) {
        ++me->other;
      }
    }
  }
  return 0;
}

static void test_two_threads_one_session(void) {
  gate_t gate = gate_new();
  locsim_session_t session = open_session(FIXED, 64);
  CHECK_EQ(locsim_session_start(session, 0), LOCSIM_OK);
  sharer_t sharers[2];
  HANDLE threads[2];
  for (int i = 0; i < 2; ++i) {
    memset(&sharers[i], 0, sizeof sharers[i]);
    sharers[i].gate = &gate;
    sharers[i].session = session;
    sharers[i].reads_too = i;
    threads[i] = CreateThread(NULL, 0, share_session, &sharers[i], 0, NULL);
  }
  gate_open(&gate, 2);
  WaitForMultipleObjects(2, threads, TRUE, INFINITE);
  for (int i = 0; i < 2; ++i) CloseHandle(threads[i]);

  /* Every call either ran or was refused as busy: nothing else. */
  CHECK_EQ(sharers[0].other, 0);
  CHECK_EQ(sharers[1].other, 0);
  /* The session is whole, and every sample it counted reached exactly one
   * of the two threads. A poll that was refused as busy did not happen, so
   * ticks can have been missed: the session may say it is falling behind,
   * but no run failed. */
  locsim_health_t health = health_of(session);
  CHECK_EQ(health.lifecycle, LOCSIM_STATE_RUNNING);
  CHECK(health.health == LOCSIM_HEALTH_HEALTHY || health.health == LOCSIM_HEALTH_DEGRADED);
  CHECK_EQ(health.causes & ~(uint32_t)(LOCSIM_CAUSE_FALLING_BEHIND | LOCSIM_CAUSE_STALLED), 0);
  CHECK_EQ(health.total_runs_started, 1);
  CHECK_EQ(health.total_samples, sharers[0].samples + sharers[1].samples);
  CHECK_EQ(health.total_failures, 0);
  CHECK_EQ(health.total_withheld, 0);
  CHECK(health.total_samples > 0);
  printf("    two threads, one session: %ld calls ran, %ld were refused as busy "
         "(not required to be more than 0), %ld samples\n",
         sharers[0].ok + sharers[1].ok, sharers[0].busy + sharers[1].busy,
         sharers[0].samples + sharers[1].samples);
  CHECK_EQ(locsim_session_destroy(session), LOCSIM_OK);
  gate_free(&gate);
}

/* Destroy while another thread is using the session. */
typedef struct {
  const gate_t *gate;
  locsim_session_t session;
  volatile LONG *destroyer_done;
  long calls, busy, other;
  locsim_status_t last;
} racer_t;

static DWORD WINAPI use_until_gone(LPVOID argument) {
  racer_t *me = (racer_t *)argument;
  gate_wait(me->gate);
  /* Until the handle is refused. If the destroyer has finished and the
   * handle still works, something is wrong: stop soon, and the checks
   * after the threads say what. */
  long after_destroyer = 0;
  for (int64_t k = 0; after_destroyer < 1000; ++k) {
    if (InterlockedCompareExchange(me->destroyer_done, 0, 0)) ++after_destroyer;
    polled_t polled = poll_at(me->session, k);
    ++me->calls;
    me->last = polled.status;
    if (polled.status == LOCSIM_ERR_INVALID_HANDLE) {
      CHECK_EQ(polled.has, BYTE_BEFORE);
      CHECK(unwritten(&polled.sample, sizeof polled.sample, sizeof polled.sample));
      return 0;
    }
    if (polled.status == LOCSIM_ERR_BUSY) {
      ++me->busy;
    } else if (polled.status != LOCSIM_OK) {
      ++me->other;
    }
  }
  return 0;
}

static DWORD WINAPI destroy_it(LPVOID argument) {
  racer_t *me = (racer_t *)argument;
  gate_wait(me->gate);
  /* Busy means not destroyed: ask again. */
  for (int64_t k = 0; k < 50000000; ++k) {
    me->last = locsim_session_destroy(me->session);
    ++me->calls;
    if (me->last != LOCSIM_ERR_BUSY) break;
    ++me->busy;
  }
  InterlockedExchange(me->destroyer_done, 1);
  return 0;
}

static void test_destroy_during_use(void) {
  long busy_destroys = 0, calls = 0;
  for (int round = 0; round < 300; ++round) {
    gate_t gate = gate_new();
    locsim_session_t session = open_session(FIXED, 8);
    CHECK_EQ(locsim_session_start(session, 0), LOCSIM_OK);
    racer_t user, destroyer;
    memset(&user, 0, sizeof user);
    memset(&destroyer, 0, sizeof destroyer);
    volatile LONG done = 0;
    user.gate = destroyer.gate = &gate;
    user.session = destroyer.session = session;
    user.destroyer_done = destroyer.destroyer_done = &done;
    HANDLE threads[2];
    threads[0] = CreateThread(NULL, 0, use_until_gone, &user, 0, NULL);
    threads[1] = CreateThread(NULL, 0, destroy_it, &destroyer, 0, NULL);
    gate_open(&gate, 2);
    WaitForMultipleObjects(2, threads, TRUE, INFINITE);
    CloseHandle(threads[0]);
    CloseHandle(threads[1]);
    gate_free(&gate);

    /* The destroy succeeded, exactly once; the user saw only what the
     * header allows and ended on an invalid handle. */
    CHECK_EQ(destroyer.last, LOCSIM_OK);
    CHECK_EQ(user.last, LOCSIM_ERR_INVALID_HANDLE);
    CHECK_EQ(user.other, 0);
    CHECK_EQ(locsim_session_destroy(session), LOCSIM_ERR_INVALID_HANDLE);
    busy_destroys += destroyer.busy;
    calls += user.calls;
  }
  printf("    destroy during use, 300 rounds: %ld polls raced; destroy was told "
         "busy %ld times (not required to be more than 0)\n",
         calls, busy_destroys);
}

/* Sessions of their own, on threads of their own. */
typedef struct {
  const gate_t *gate;
  uint64_t hash;
} loner_t;

static DWORD WINAPI run_alone(LPVOID argument) {
  loner_t *me = (loner_t *)argument;
  gate_wait(me->gate);
  for (int n = 0; n < 20; ++n) me->hash = run_by_deadline(DRIVING, NULL);
  return 0;
}

static void test_independent_sessions(void) {
  enum { THREADS = 8 };
  const uint64_t expected = run_by_deadline(DRIVING, NULL);
  gate_t gate = gate_new();
  loner_t loners[THREADS];
  HANDLE threads[THREADS];
  for (int i = 0; i < THREADS; ++i) {
    loners[i].gate = &gate;
    loners[i].hash = 0;
    threads[i] = CreateThread(NULL, 0, run_alone, &loners[i], 0, NULL);
  }
  gate_open(&gate, THREADS);
  WaitForMultipleObjects(THREADS, threads, TRUE, INFINITE);
  for (int i = 0; i < THREADS; ++i) {
    CloseHandle(threads[i]);
    /* No session was disturbed by the others: the same samples as alone. */
    CHECK(loners[i].hash == expected);
  }
  gate_free(&gate);
}

/* ---- Main ---------------------------------------------------------------- */

static void run(const char *name, void (*test)(void)) {
  LONG before = g_failures;
  LONG checks = g_checks;
  test();
  printf("%s %s (%ld checks)\n", g_failures == before ? "ok    " : "FAILED", name,
         (long)(g_checks - checks));
  fflush(stdout);
}

static const char *g_dump_path;
static void test_dump_to_path(void) { test_dump(g_dump_path); }

int main(int argc, char **argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: harness <scenario directory> <samples file>\n");
    return 2;
  }
  if (!load_examples(argv[1])) return 2;
  g_dump_path = argv[2];

  printf("locsim header version %u, library version %u\n", (unsigned)LOCSIM_ABI_VERSION,
         (unsigned)locsim_abi_version());
  printf("sizeof: policy %u sample %u health %u event %u\n",
         (unsigned)sizeof(locsim_policy_t), (unsigned)sizeof(locsim_sample_t),
         (unsigned)sizeof(locsim_health_t), (unsigned)sizeof(locsim_event_t));

  run("version", test_version);
  run("create and destroy", test_create_and_destroy);
  run("lifecycle", test_lifecycle);
  run("samples of a scenario", test_samples);
  run("health", test_health);
  run("faults", test_faults);
  run("events", test_events);
  run("refused calls write nothing", test_refusals);
  run("every example by its deadlines", test_dump_to_path);
  run("create, use, destroy cycles and the limit", test_cycles);
  run("two threads on one session", test_two_threads_one_session);
  run("destroy during use", test_destroy_during_use);
  run("independent sessions on eight threads", test_independent_sessions);

  printf("%ld checks, %ld failed\n", (long)g_checks, (long)g_failures);
  return g_failures == 0 ? 0 : 1;
}
