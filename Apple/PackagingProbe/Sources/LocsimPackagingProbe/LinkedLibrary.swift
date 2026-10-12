import LocsimFFI

/// The ABI version reported by the library this target was linked with.
///
/// A fixture: it exists so that a Swift target other than the tests links
/// the packaged library. It is not part of any adapter.
public func linkedABIVersion() -> UInt32 {
    locsim_abi_version()
}
