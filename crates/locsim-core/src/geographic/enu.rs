//! Local tangent plane (East-North-Up) frames via ECEF.
//!
//! Movement models compute in a metric ENU frame anchored at the scenario
//! origin and convert back to WGS84, instead of adding offsets to degrees.

use super::coordinate::Coordinate;
use super::wgs84::{A, B, E2};
use super::GeoError;

/// Metres east, north and up of a frame origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Enu {
    pub east: f64,
    pub north: f64,
    pub up: f64,
}

impl Enu {
    pub fn horizontal_distance(&self) -> f64 {
        self.east.hypot(self.north)
    }
}

/// Earth-centred, Earth-fixed Cartesian position (metres) of a geodetic
/// point at `altitude_m` above the ellipsoid.
pub fn geodetic_to_ecef(coord: Coordinate, altitude_m: f64) -> [f64; 3] {
    let (sp, cp) = coord.latitude().to_radians().sin_cos();
    let (sl, cl) = coord.longitude().to_radians().sin_cos();
    let n = A / (1.0 - E2 * sp * sp).sqrt();
    [
        (n + altitude_m) * cp * cl,
        (n + altitude_m) * cp * sl,
        (n * (1.0 - E2) + altitude_m) * sp,
    ]
}

/// Inverse of [`geodetic_to_ecef`]: the geodetic point beneath (or above)
/// an ECEF position, and its height over the ellipsoid.
pub fn ecef_to_geodetic(p: [f64; 3]) -> Result<(Coordinate, f64), GeoError> {
    let [x, y, z] = p;
    let rho = x.hypot(y);
    if rho < 1e-6 {
        // On the polar axis longitude is undefined; 0 is the canonical choice.
        let lat = if z >= 0.0 { 90.0 } else { -90.0 };
        return Ok((Coordinate::new(lat, 0.0)?, z.abs() - B));
    }
    let lon = y.atan2(x);
    let mut lat = z.atan2(rho * (1.0 - E2));
    let mut alt = 0.0;
    for _ in 0..20 {
        let (sp, cp) = lat.sin_cos();
        let n = A / (1.0 - E2 * sp * sp).sqrt();
        // Pick the better-conditioned expression for height near the poles.
        alt = if cp.abs() > 0.5 {
            rho / cp - n
        } else {
            z / sp - n * (1.0 - E2)
        };
        let next = z.atan2(rho * (1.0 - E2 * n / (n + alt)));
        let done = (next - lat).abs() < 1e-15;
        lat = next;
        if done {
            break;
        }
    }
    if !lat.is_finite() || !alt.is_finite() {
        return Err(GeoError::InvalidInput(
            "ECEF point has no geodetic solution",
        ));
    }
    let coord = Coordinate::new(lat.to_degrees().clamp(-90.0, 90.0), lon.to_degrees())?;
    Ok((coord, alt))
}

/// Unit vectors, in ECEF, pointing east, north and up at a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalAxes {
    pub east: [f64; 3],
    pub north: [f64; 3],
    pub up: [f64; 3],
}

/// The local east/north/up directions at `coord`. At a pole east and north
/// follow the coordinate's stated longitude.
pub fn local_axes(coord: Coordinate) -> LocalAxes {
    let (sin_lat, cos_lat) = coord.latitude().to_radians().sin_cos();
    let (sin_lon, cos_lon) = coord.longitude().to_radians().sin_cos();
    LocalAxes {
        east: [-sin_lon, cos_lon, 0.0],
        north: [-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat],
        up: [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat],
    }
}

/// An ENU frame anchored at a geodetic origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnuFrame {
    origin: Coordinate,
    origin_altitude_m: f64,
    ecef: [f64; 3],
    sin_lat: f64,
    cos_lat: f64,
    sin_lon: f64,
    cos_lon: f64,
}

impl EnuFrame {
    pub fn new(origin: Coordinate, origin_altitude_m: f64) -> Result<Self, GeoError> {
        if !origin_altitude_m.is_finite() {
            return Err(GeoError::InvalidInput("origin altitude must be finite"));
        }
        let (sin_lat, cos_lat) = origin.latitude().to_radians().sin_cos();
        let (sin_lon, cos_lon) = origin.longitude().to_radians().sin_cos();
        Ok(Self {
            origin,
            origin_altitude_m,
            ecef: geodetic_to_ecef(origin, origin_altitude_m),
            sin_lat,
            cos_lat,
            sin_lon,
            cos_lon,
        })
    }

    pub fn origin(&self) -> Coordinate {
        self.origin
    }

    pub fn origin_altitude_m(&self) -> f64 {
        self.origin_altitude_m
    }

    pub fn to_enu(&self, coord: Coordinate, altitude_m: f64) -> Result<Enu, GeoError> {
        if !altitude_m.is_finite() {
            return Err(GeoError::InvalidInput("altitude must be finite"));
        }
        let p = geodetic_to_ecef(coord, altitude_m);
        let (dx, dy, dz) = (
            p[0] - self.ecef[0],
            p[1] - self.ecef[1],
            p[2] - self.ecef[2],
        );
        Ok(Enu {
            east: -self.sin_lon * dx + self.cos_lon * dy,
            north: -self.sin_lat * self.cos_lon * dx - self.sin_lat * self.sin_lon * dy
                + self.cos_lat * dz,
            up: self.cos_lat * self.cos_lon * dx
                + self.cos_lat * self.sin_lon * dy
                + self.sin_lat * dz,
        })
    }

    /// Exact inverse of [`EnuFrame::to_enu`]: returns coordinate and altitude.
    pub fn from_enu(&self, enu: Enu) -> Result<(Coordinate, f64), GeoError> {
        if !(enu.east.is_finite() && enu.north.is_finite() && enu.up.is_finite()) {
            return Err(GeoError::InvalidInput("ENU components must be finite"));
        }
        let Enu {
            east: e,
            north: n,
            up: u,
        } = enu;
        let dx =
            -self.sin_lon * e - self.sin_lat * self.cos_lon * n + self.cos_lat * self.cos_lon * u;
        let dy =
            self.cos_lon * e - self.sin_lat * self.sin_lon * n + self.cos_lat * self.sin_lon * u;
        let dz = self.cos_lat * n + self.sin_lat * u;
        ecef_to_geodetic([self.ecef[0] + dx, self.ecef[1] + dy, self.ecef[2] + dz])
    }

    /// Maps a horizontal offset to the coordinate beneath/above it, discarding
    /// the height the flat tangent plane gains over the curved ellipsoid
    /// (≈ d²/2R: 8 cm at 1 km). Use for ground movement at constant altitude.
    pub fn horizontal_to_coordinate(&self, east: f64, north: f64) -> Result<Coordinate, GeoError> {
        self.from_enu(Enu {
            east,
            north,
            up: 0.0,
        })
        .map(|(c, _)| c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geographic::{destination, distance};

    fn c(lat: f64, lon: f64) -> Coordinate {
        Coordinate::new(lat, lon).unwrap()
    }

    #[test]
    fn origin_maps_to_zero() {
        let f = EnuFrame::new(c(12.9352, 77.6245), 920.0).unwrap();
        let e = f.to_enu(f.origin(), 920.0).unwrap();
        assert!(e.east.abs() < 1e-9 && e.north.abs() < 1e-9 && e.up.abs() < 1e-9);
    }

    #[test]
    fn axes_point_east_north_up() {
        let o = c(12.9352, 77.6245);
        let f = EnuFrame::new(o, 0.0).unwrap();
        let east = f.to_enu(destination(o, 90.0, 100.0).unwrap(), 0.0).unwrap();
        assert!((east.east - 100.0).abs() < 1e-3 && east.north.abs() < 1e-3);
        let north = f.to_enu(destination(o, 0.0, 100.0).unwrap(), 0.0).unwrap();
        assert!((north.north - 100.0).abs() < 1e-3 && north.east.abs() < 1e-3);
        let up = f.to_enu(o, 50.0).unwrap();
        assert!((up.up - 50.0).abs() < 1e-9 && up.horizontal_distance() < 1e-9);
    }

    #[test]
    fn round_trip_with_altitude() {
        let f = EnuFrame::new(c(-33.8688, 151.2093), 30.0).unwrap();
        let enu = Enu {
            east: 1234.5,
            north: -987.6,
            up: 42.0,
        };
        let (coord, alt) = f.from_enu(enu).unwrap();
        let back = f.to_enu(coord, alt).unwrap();
        assert!((back.east - enu.east).abs() < 1e-6);
        assert!((back.north - enu.north).abs() < 1e-6);
        assert!((back.up - enu.up).abs() < 1e-6);
    }

    #[test]
    fn works_at_poles_and_antimeridian() {
        for origin in [
            c(90.0, 0.0),
            c(-90.0, 0.0),
            c(0.0, 180.0),
            c(89.9999, -180.0),
        ] {
            let f = EnuFrame::new(origin, 0.0).unwrap();
            let p = f.horizontal_to_coordinate(250.0, -400.0).unwrap();
            let d = distance(origin, p).unwrap();
            assert!((d - 250f64.hypot(400.0)).abs() < 1e-2, "{origin:?}: {d}");
        }
        // The polar axis itself converts cleanly.
        let f = EnuFrame::new(c(90.0, 0.0), 0.0).unwrap();
        let (p, alt) = f
            .from_enu(Enu {
                east: 0.0,
                north: 0.0,
                up: 10.0,
            })
            .unwrap();
        assert_eq!(p.latitude(), 90.0);
        assert!((alt - 10.0).abs() < 1e-6);
    }

    #[test]
    fn ecef_round_trip_and_local_axes() {
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for (lat, lon, alt) in [
            (12.9352, 77.6245, 920.0),
            (-45.0, 179.9, 0.0),
            (89.999, -30.0, -50.0),
            (0.0, 0.0, 0.0),
        ] {
            let p = c(lat, lon);
            let x = geodetic_to_ecef(p, alt);
            let (back, height) = ecef_to_geodetic(x).unwrap();
            assert!((back.latitude() - lat).abs() < 1e-10 && (back.longitude() - lon).abs() < 1e-9);
            assert!((height - alt).abs() < 1e-6);

            // Orthonormal, and consistent with the ENU frame at that point.
            let axes = local_axes(p);
            for (u, v) in [
                (axes.east, axes.north),
                (axes.east, axes.up),
                (axes.north, axes.up),
            ] {
                assert!(dot(u, v).abs() < 1e-15);
            }
            for u in [axes.east, axes.north, axes.up] {
                assert!((dot(u, u) - 1.0).abs() < 1e-15);
            }
            // One metre up really is one metre of altitude.
            let above = [x[0] + axes.up[0], x[1] + axes.up[1], x[2] + axes.up[2]];
            assert!((ecef_to_geodetic(above).unwrap().1 - alt - 1.0).abs() < 1e-6);
            // One metre north raises the latitude and keeps the longitude.
            let north = [
                x[0] + axes.north[0],
                x[1] + axes.north[1],
                x[2] + axes.north[2],
            ];
            let (moved, _) = ecef_to_geodetic(north).unwrap();
            assert!(moved.latitude() > lat);
            if lat.abs() < 89.0 {
                assert!((moved.longitude() - lon).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn rejects_non_finite() {
        let o = c(0.0, 0.0);
        assert!(EnuFrame::new(o, f64::NAN).is_err());
        let f = EnuFrame::new(o, 0.0).unwrap();
        assert!(f.to_enu(o, f64::INFINITY).is_err());
        assert!(f.horizontal_to_coordinate(f64::NAN, 0.0).is_err());
    }
}
