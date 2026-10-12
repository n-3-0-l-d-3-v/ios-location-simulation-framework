import Foundation
import LocsimFFI
import LocsimPackagingProbe
import XCTest

/// What these tests show when they pass: the packaged library was linked
/// into a Swift test bundle, loaded, and answered four calls. They do not
/// test the simulation, the adapter, Core Location or a running app.
final class PackagingProbeTests: XCTestCase {
    /// The policy of the C harness, `good_policy`.
    private func policy() -> locsim_policy_t {
        var policy = locsim_policy_t()
        policy.struct_size = UInt32(MemoryLayout<locsim_policy_t>.size)
        policy.max_recovery_attempts = 3
        policy.backoff_initial_s = 1.0
        policy.backoff_multiplier = 2.0
        policy.backoff_max_s = 60.0
        policy.stable_after_s = 10.0
        policy.stall_after_s = 5.0
        policy.missed_window_ticks = 10
        policy.max_missed_in_window = 2
        return policy
    }

    private func create(_ document: Data) -> (
        status: locsim_status_t, session: locsim_session_t, errorLength: Int
    ) {
        var policy = policy()
        var session: locsim_session_t = 0
        var errorLength = 0
        let status = document.withUnsafeBytes { bytes in
            locsim_session_create(
                bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count,
                &policy, 16, &session, nil, 0, &errorLength)
        }
        return (status, session, errorLength)
    }

    /// Where this ran, written to the log as evidence. Fails anywhere but
    /// an iOS Simulator, so a run on the build host cannot pass for one.
    func testItRunsInAnIOSSimulator() {
        let environment = ProcessInfo.processInfo.environment
        print(
            "LOCSIM-PROBE os=\"\(ProcessInfo.processInfo.operatingSystemVersionString)\""
                + " model=\(environment["SIMULATOR_MODEL_IDENTIFIER"] ?? "none")"
                + " runtime=\(environment["SIMULATOR_RUNTIME_VERSION"] ?? "none")"
                + " abi=\(locsim_abi_version())")
        #if targetEnvironment(simulator) && os(iOS)
        #else
        XCTFail("not an iOS Simulator")
        #endif
    }

    func testTheLinkedLibraryHasTheABIVersionOfTheHeader() {
        XCTAssertEqual(locsim_abi_version(), LOCSIM_ABI_VERSION)
        XCTAssertEqual(linkedABIVersion(), LOCSIM_ABI_VERSION)
    }

    func testASessionIsCreatedFromACheckedInScenarioAndDestroyed() throws {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "walking", withExtension: "json", subdirectory: "Fixtures"))
        let created = create(try Data(contentsOf: url))
        XCTAssertEqual(created.status, LOCSIM_OK)
        XCTAssertNotEqual(created.session, 0)
        XCTAssertEqual(created.errorLength, 0)

        XCTAssertEqual(locsim_session_destroy(created.session), LOCSIM_OK)
        // The handle is gone: the library kept state and released it.
        XCTAssertEqual(locsim_session_destroy(created.session), LOCSIM_ERR_INVALID_HANDLE)
    }

    /// A library that answered `LOCSIM_OK` to everything would pass the
    /// test above. It would fail this one.
    func testADocumentThatIsNotAScenarioIsRefused() {
        let refused = create(Data("{}".utf8))
        XCTAssertEqual(refused.status, LOCSIM_ERR_SCENARIO)
        XCTAssertEqual(refused.session, 0)
        XCTAssertGreaterThan(refused.errorLength, 0)
    }
}
