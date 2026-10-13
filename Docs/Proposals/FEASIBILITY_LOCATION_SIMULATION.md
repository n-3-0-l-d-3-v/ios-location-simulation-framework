# Feasibility study: what location simulation on iOS can and cannot reach

**Status: a study and a test plan, written on 2026-10-13. No source code
was changed for it, and nothing in it is authorised work.** Every
experiment in section 5 is a proposal.

How to read the claims. Each is marked with where it comes from:

- **[Apple]** Apple's documentation, with a link, read on 2026-10-13.
- **[Repo]** this repository, at the commit named.
- **[Run]** a recorded run, with its number.
- **[Hypothesis]** not documented and not run. Believed, not known.

## 1. State of the repository

Verified on 2026-10-13. **[Repo]**

- `main` at `c77d781`, equal to `origin/main`, working tree clean.
- T11A (the C interface) is closed. T11B (the Apple side) is not started
  and not authorised.
- The project's own notes say 594 tests and no Swift. Both are out of
  date by one day's work: 595 tests pass on Windows x86_64 and on macOS
  arm64 **[Run 38239633331]**, and three Swift files exist. They are a
  packaging fixture, not an adapter.
- That fixture was compiled, linked and run in an iPhone 16 Pro Simulator
  on iOS 18.6: ABI version, create and destroy a session, a refused
  document **[Run 38237842696]**. No sample has been polled on iOS.
- There is no `CLLocation` conversion, no GPX code, no app, no device
  build, and no run on any physical iPhone.
- One core defect is open: pause and resume
  (`Docs/Diagnostics/long_gap`).

## 2. Four different things

They are often spoken of as one. They sit at different layers, and success
at one says nothing about the next.

| | A. Generate coordinates | B. Hand them to an app that links the library | C. Simulate location in the development environment | D. Change what a system location-sharing service reports |
|---|---|---|---|---|
| What happens | Arithmetic. A scenario and a seed give positions and timestamps | The app's own code builds `CLLocation` values and uses them in place of what `CLLocationManager` would deliver | Xcode, the Simulator or a UI test tells Core Location what the location is | The location the system shares on the user's behalf would be something other than where the device is |
| Where it runs | Anywhere Rust runs | Inside one app's process | Inside the operating system's location service, under a developer tool | Inside the operating system and Apple's services |
| Who sees the result | Whoever reads the output | That app only. Its own `CLLocationManager`, other apps and the system still see the real location | Clients of Core Location, while the tool is active | The people the user shares with |
| Is Core Location involved | No | No. Only its data type is borrowed | Yes | Yes |
| Marked as simulated | Not applicable | Only if the app says so | Yes, by the system (below) | — |
| Documented interface | Not an Apple matter | Public initialisers of `CLLocation` | Yes, several (section 3) | **None found** |

Layer A is what the Rust engine does. Layer B is what T11B plans. Layer C
exists and belongs to Apple's tools. For layer D, the documentation
reviewed offers controls over *whether* and *from which device* a location
is shared, and nothing that sets *what* it is.

## 3. What Apple documents

### Building a location value (layer B)

- `CLLocation` has public initialisers taking coordinate, altitude,
  accuracies, course, speed and timestamp; from iOS 15 one also takes a
  `CLLocationSourceInformation`. **[Apple]**
  [init(coordinate:…sourceInfo:)](https://developer.apple.com/documentation/corelocation/cllocation/init(coordinate:altitude:horizontalaccuracy:verticalaccuracy:course:courseaccuracy:speed:speedaccuracy:timestamp:sourceinfo:))
- An app can therefore construct any location it likes for its own use.
  That is ordinary API use and needs no entitlement. It does not pass
  through Core Location and changes nothing outside the app.
  **[Hypothesis]** as to "needs no entitlement": no page reviewed says
  otherwise; it has not been run by this project.

### Simulation by developer tools (layer C)

- **The Xcode debugger and GPX.** "You can simulate locations by loading
  GPX files using the Xcode debugger." **[Apple]**
  [isSimulatedBySoftware](https://developer.apple.com/documentation/corelocation/cllocationsourceinformation/issimulatedbysoftware)
- **Scheme setting.** "Allow Location Simulation", with a default
  location or an added GPX file, in the scheme's run options. **[Apple]**,
  archived guide:
  [Customizing Your Simulator Experience with Xcode Schemes](https://developer.apple.com/library/archive/documentation/IDEs/Conceptual/iOS_Simulator_Guide/CustomizingYourExperienceThroughXcodeSchemes/CustomizingYourExperienceThroughXcodeSchemes.html).
  The page is archived; current Xcode's wording was not checked.
- **UI tests.** `XCUIDevice.shared.location` takes an `XCUILocation`, "the
  proxy location a test uses to simulate longitude, latitude, and course
  information for the device"; without one, "a test uses the physical
  location of the device that Core Location provides". iOS 16.4 and
  later. **[Apple]**
  [XCUIDevice.location](https://developer.apple.com/documentation/xcuiautomation/xcuidevice/location)
- **`simctl location`,** for a Simulator: `set` a coordinate, `run` a
  built-in scenario, `start` a list of waypoints that the Simulator
  interpolates at one speed, `clear`. **It takes no GPX file and no
  timestamps.** **[Run 38239633331]**: this is the tool's own help text on
  Xcode 16.4, not a documentation page.
- **Simulated locations are labelled.** `isSimulatedBySoftware` is "true
  if the system generated the location using on-device software
  simulation", and Apple's example is an app that chooses to "reject
  locations if the `isSimulatedBySoftware` property is `true` when the
  developer isn't debugging or testing". **[Apple]**
  [CLLocationSourceInformation](https://developer.apple.com/documentation/corelocation/cllocationsourceinformation)

What the reviewed pages do **not** say: which processes on a physical
device receive a location simulated by Xcode, whether system services do,
and what happens when the debug session ends. Those are undocumented here.

### Permissions

- An app asks for When In Use or Always authorisation, and the user can
  grant precise or approximate accuracy. **[Apple]**
  [Requesting authorization to use location services](https://developer.apple.com/documentation/corelocation/requesting-authorization-to-use-location-services),
  [Choosing the Location Services Authorization to Request](https://developer.apple.com/documentation/bundleresources/choosing-the-location-services-authorization-to-request),
  [NSLocationDefaultAccuracyReduced](https://developer.apple.com/documentation/bundleresources/information-property-list/nslocationdefaultaccuracyreduced)
- Layer B needs none of this: the app is not asking the system for a
  location. An app that also wants the real one, for comparison, does.

### Find My and location sharing (layer D)

- Sharing is tied to a device: "The device sharing your location appears
  next to Sharing From", and a user can "Use This iPhone as My Location".
  A user can stop sharing with one person, hide their location, or turn
  Share My Location off. **[Apple]**
  [Share your location in Find My on iPhone](https://support.apple.com/guide/iphone/share-your-location-iph01954dc44/ios),
  [Share your location with your Family Sharing group](https://support.apple.com/en-us/105107),
  [Find My and location sharing](https://support.apple.com/guide/personal-safety/find-my-and-location-sharing-ips05ede4573/web)
- Find My needs Location Services to be on for it. **[Apple]**, same guide.
- How it is protected in transit and at rest: **[Apple]**
  [Find My security](https://support.apple.com/guide/security/find-my-security-sec6cbc80fd0/web).
  Found by search; not read in full for this study.
- **No page reviewed describes an interface by which an app, a library or
  a developer tool sets the location Find My shares.** The documented
  controls are the ones above: share or not, with whom, from which device.

## 4. What is supported where

| Capability | Ordinary iPhone, no development setup | Needs a development environment | Status for this project |
|---|---|---|---|
| A. Generate a trajectory | Not applicable: no phone involved | Rust toolchain | Done, tested on Windows and macOS hosts **[Run]** |
| B. An app using locations it built itself | Yes in principle: public API. The app must be installed, which itself needs signing | To build and sign it: a Mac, Xcode, a signing identity | Not built. Unverified on a Simulator and on a device |
| C. Simulated location in a Simulator | Not applicable | Mac with Xcode | Documented. Not run by this project beyond booting a Simulator |
| C. Simulated location on a physical iPhone from Xcode | No | Mac, Xcode, the device attached with Developer Mode, an app being debugged | Documented to exist. Scope and lifetime on the device undocumented. Not run |
| C. `XCUIDevice.location` in a UI test | No | A UI test run from Xcode | Documented. Not run |
| An app telling a simulated location from a real one | Yes: `isSimulatedBySoftware`, iOS 15 and later | — | Documented. Not run |
| D. Setting what Find My shares | **No documented way** | **No documented way** | Out of scope (below). Not tested, and no test is proposed |
| Limiting what is shared: stop, hide, choose the device, approximate accuracy for apps | Yes, in Settings and Find My | — | Documented; these are user controls, not something a library does |

### On layer D

The honest finding is negative and short. A library inside an app (layer
B) cannot reach Find My: it never touches the system's location. A
developer tool (layer C) acts on Core Location while a development session
is active and labels its output as simulated; whether a system service
honours it is not documented, and it is a debugging aid, not a supported
way to share a location. This project's own fixed boundary (contract C18,
and the scope the owner restated for this work) excludes changing what
Find My reports, and making a location-sharing service show people a
place the device is not is not a goal this study will plan towards. If the
underlying need is privacy, the supported answers are the controls in the
last row of the table.

## 5. Experiment matrix

Documented interfaces only. All on the project's own test app and, for
E6 and E7, a development device whose owner has agreed. None has been run.

| ID | Hypothesis | Setup | Measurements | Expected | A pass proves | A pass does not prove |
|---|---|---|---|---|---|---|
| E1 | A `locsim_sample_t` converts to a `CLLocation` without loss or invention | Swift unit tests in an iOS Simulator, on the existing fixture package | Each field against the sample; unknown speed and course; `CLLocationCoordinate2DIsValid`; timestamp by the agreed rule | Equal values; unknown speed and course are negative, never 0 | The conversion is right for the cases tested, on that runtime | Anything about delivery, timing, or a device |
| E2 | A scenario polled through the C interface on iOS gives the stream the reference gives | Fixture extended to poll N samples of a named scenario at its deadlines, in the Simulator | Every sample against `reference.rs` output made on the same kind of host | Equal on arm64 macOS reference; last-digit differences against a Windows reference **[Run 38238724688]** | The engine runs correctly inside an iOS Simulator process | Real-time behaviour: nothing sleeps |
| E3 | `simctl location set` reaches an app's `CLLocationManager`, labelled as simulated | A minimal test app with When In Use authorisation, in a booted Simulator | Coordinate received; `sourceInformation?.isSimulatedBySoftware`; delay | The set coordinate; flag true | Layer C works for a Simulator app on that runtime, and how it is labelled | Anything about a device. Whether the flag is set in a Simulator is itself unverified: a false here is a finding, not a failure of the project |
| E4 | A trajectory from the engine can be replayed through `simctl location start` | Waypoints exported from a scenario, piped to `simctl`; same app as E3 | Sequence of positions; spacing in time against the scenario's | Positions on the path; **timing not preserved**, since `simctl` interpolates at one speed | What of a scenario survives this route: the path, not its timing or speed profile | That GPX works: `simctl` takes none |
| E5 | A GPX file written from a scenario is accepted by Xcode and moves the location as the file says | GPX export (does not exist yet), a scheme with Allow Location Simulation, run from Xcode on a Simulator | File against the GPX schema; positions and times received by the app | To be found out. How Xcode treats `<time>` is not in the pages reviewed | Whether GPX is a usable delivery route, on that Xcode | Not automatable on hosted CI without checking: it may need Xcode's interface |
| E6 | Layer B on a real iPhone gives the same stream as in the Simulator, and leaves the system's location alone | Test app with the adapter, signed, on a development iPhone. It shows the adapter's stream and, separately, its own `CLLocationManager` | Adapter samples against the reference; the real `CLLocationManager` location before, during, after | Adapter stream as E2; real location unchanged throughout | The adapter works on that device and iOS build, and is confined to the app | Other devices or builds. Nothing about any other app or service |
| E7 | Xcode's Simulate Location on a physical iPhone is delivered to the project's own app, labelled, and ends with the session | Same app, debug session from Xcode with a GPX file or fixed location | Location and `isSimulatedBySoftware` in the app during the session; the same after the session is stopped; time to return to the real location | Simulated and flagged during; real after | How the documented debug feature behaves for the app under test on that iOS build | Scope beyond the app under test. It is observed in the test app only |
| E8 | A UI test's `XCUIDevice.shared.location` reaches the app | UI test target against the test app, Simulator | Coordinate and flag received | The set coordinate | A scriptable, documented route for automated tests | That course or speed are honoured: only what is measured |

Order of cost: E1 and E2 need only what already runs in CI. E3, E4 and E8
need a test app. E5 probably needs a Mac in hand. E6 and E7 need a Mac, a
signing identity and a device.

## 6. Is the engine a useful foundation

**Yes for layers A and B, and as a source of test data for layer C. It is
not, and cannot be made into, a route to layer D.**

What it already gives **[Repo]**: reproducible trajectories from a
scenario and a seed; a validation gate; a supervised session behind 15 C
functions; a packaging route that a Swift target has linked and called in
a Simulator.

Main gaps, in the order they block:

1. No Swift adapter and no `CLLocation` conversion. The rule for turning
   the caller's time scale into a `Date` is undecided.
2. No driver: nothing calls `poll` and `check` on a clock, and behaviour
   on suspension is undecided.
3. The pause/resume defect, and the collapsed speed on the first sample
   after any resume. Any real driver will pause.
4. Numbers differ in their last digits between Windows and macOS. A
   reference stream must be made on the kind of host it is compared on.
5. No GPX export. `simctl` cannot take a GPX file, so GPX matters only
   for Xcode's own replay, and E5 should be run before investing in it.
6. Only a Simulator slice exists. A device needs `aarch64-apple-ios`,
   signing, and a Mac in hand.
7. `nm` cannot read Rust's standard-library bitcode with Xcode 16.4; a
   consumer built with link-time optimisation is untested.

**Next smallest milestone: E1.** `locsim_sample_t` to `CLLocation`, as
Swift tests in the Simulator, inside the existing fixture and workflow. It
needs two decisions first (the timestamp rule; the minimum iOS version),
changes no public API, and is the first criterion of T11B that every
later experiment depends on. E2 follows with no new infrastructure. Both
need authorisation: they are the start of T11B.

## 7. Compatibility strategy

The full proposal is `Docs/Proposals/IOS_COMPATIBILITY_POLICY.md`. In
short:

- A configuration is an iOS version, a device or Simulator type, an
  Xcode, a Rust and a commit. It is Supported only when every mandatory
  test has a recorded pass on it. Building is never enough.
- Simulator and physical device are separate configurations with separate
  statuses.
- One pinned baseline gates releases (today: iPhone 16 Pro Simulator, iOS
  18.6, Xcode 16.4, Rust 1.98.1). A canary on the newest Xcode and stable
  Rust runs weekly and blocks nothing. Betas are run by hand and never
  block.
- CI uses runtimes already on the runner and downloads none. When a
  pinned piece leaves the hosted image, the row becomes Untested, not
  silently kept.
- Physical-device rows change only on a recorded run on that device and
  build. An iPhone 16 on iOS 18.7.8 is Untested; no 18.7 Simulator
  runtime was on the runner, so only a device run can cover it.

This study adds three points to that proposal:

- Experiments E3, E7 and E8 depend on Apple's tools, not on this
  project's code. Their results can change with any Xcode or iOS release
  and should be re-run per release as observations, kept apart from the
  project's own pass/fail rows.
- `isSimulatedBySoftware` should be recorded in every layer C experiment.
  It is the documented signal by which apps tell simulated from real, and
  the project's documents must go on saying that simulated locations are
  identifiable (contract C18).
- No row for any system service. The matrix covers the project's own app
  and nothing else.

## 8. Decisions asked for

1. Authorise E1, and then E2, as the first work of T11B, or not.
2. The rule for `CLLocation.timestamp`, and the minimum iOS version.
3. Whether a minimal test app (needed by E3, E4, E8) is in scope before
   the adapter exists.
4. Whether a Mac, a signing identity and a development device will be
   available, which decides whether E5 to E7 can ever run.
5. Whether GPX stays in T11B at all if E5 shows Xcode does not honour a
   file's timing.
