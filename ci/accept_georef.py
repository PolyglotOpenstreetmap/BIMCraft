"""Independent CLI -> native save/reopen -> IFC acceptance against PROJ and IfcOpenShell.

Usage: python ci/accept_georef.py PATH/TO/bimcraft-cli OUTPUT_DIRECTORY
Requires ifcopenshell==0.8.5, pyproj==3.7.2 and pytest (for EXPRESS rules).
All locations and geometry are synthetic. No network or private input is used.
"""

import json
import math
from pathlib import Path
import subprocess
import sys

import ifcopenshell
import ifcopenshell.geom
import ifcopenshell.util.geolocation as geo
import ifcopenshell.util.shape
import ifcopenshell.util.unit
import ifcopenshell.validate
import pyproj


def close(actual, expected, tolerance, label):
    assert abs(actual - expected) <= tolerance, f"{label}: {actual} != {expected} (tolerance {tolerance})"


def run(cli, *args):
    result = subprocess.run([cli, *map(str, args)], capture_output=True, text=True, check=True)
    return [json.loads(line) for line in result.stdout.splitlines() if line.strip()]


def check(cli, out, case):
    name = case["name"]
    location = case["location"]
    lon, lat = location["longitude"], location["latitude"]
    angle = location["true_north"]
    projection = pyproj.Proj(location["crs"])
    factors = projection.get_factors(lon, lat)
    easting, northing = projection(lon, lat)
    rotation = math.radians(factors.meridian_convergence - angle)
    scale = factors.parallel_scale
    # Recompute the frozen test oracle, so the Rust tests' constants remain independently reproducible.
    for key, actual, tolerance in [
        ("easting", easting, 1e-6), ("northing", northing, 1e-6),
        ("convergence", factors.meridian_convergence, 1e-8), ("scale", scale, 1e-10),
    ]:
        close(actual, case[key], tolerance, name + " oracle " + key)
    native, ifc = out / (name + ".bimcraft"), out / (name + ".ifc")
    script = out / (name + ".txt")
    script.write_text('wall {"points":[[0,0],[10000,0]],"height":3000,"type":"Interior - 100mm"}\n'
                      + "geolocate " + json.dumps(location) + "\n", encoding="utf-8")
    run(cli, "run", "--script", script, "--save", native)
    # A fresh process must see the saved location and produce the IFC.
    reopened = run(cli, "run", native, "--cmd", "geolocate {}", "--save", ifc)
    located = next(value for value in reopened if value.get("located"))
    close(located["crs"]["convergence"], factors.meridian_convergence, 1e-6, name + " CLI convergence")
    f = ifcopenshell.open(ifc)
    logger = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(f, logger, express_rules=True)
    assert not logger.statements, logger.statements
    ids = [entity.GlobalId for entity in f.by_type("IfcRoot")]
    assert len(ids) == len(set(ids)), "duplicate GlobalIds"
    site, = f.by_type("IfcSite")
    def degrees(parts):
        return sum(value / unit for value, unit in zip(parts, (1, 60, 3600, 3.6e9)))
    close(degrees(site.RefLatitude), lat, 1e-9, name + " latitude")
    close(degrees(site.RefLongitude), lon, 1e-9, name + " longitude")
    close(site.RefElevation, location["elevation"], 1e-6, name + " site elevation in mm")
    close(geo.get_true_north(f), angle, 1e-7, name + " TrueNorth")
    conversion, = f.by_type("IfcMapConversion")
    assert conversion.TargetCRS.Name == f'EPSG:{location["crs"]}'
    assert conversion.TargetCRS.MapUnit.Name == "METRE" and conversion.TargetCRS.MapUnit.Prefix is None
    close(ifcopenshell.util.unit.calculate_unit_scale(f), 0.001, 1e-12, name + " project millimetres")
    close(conversion.Eastings, easting, 1e-3, name + " eastings")
    close(conversion.Northings, northing, 1e-3, name + " northings")
    close(conversion.OrthogonalHeight, location["elevation"] / 1000, 1e-9, name + " map elevation in m")
    close(conversion.XAxisAbscissa, math.cos(rotation), 2e-8, name + " rotation cosine")
    close(conversion.XAxisOrdinate, math.sin(rotation), 2e-8, name + " rotation sine")
    close(conversion.Scale, scale / 1000, 1e-10, name + " mm-to-grid scale")
    # Independent application of IFC map conversion: corners and a kilometre-long true-north ray.
    for x, y in [(0, 0), (10000, 0), (10000, 10000), (-250000, 125000),
                 (-math.sin(math.radians(angle)) * 1e6, math.cos(math.radians(angle)) * 1e6)]:
        e, n, _ = geo.auto_xyz2enh(f, x, y, 0)
        close(e, easting + scale / 1000 * (x * math.cos(rotation) - y * math.sin(rotation)), 1e-3, name + " mapped easting")
        close(n, northing + scale / 1000 * (x * math.sin(rotation) + y * math.cos(rotation)), 1e-3, name + " mapped northing")
    # Check the final ray independently as a geodesic bearing after inverse projection.
    end_lon, end_lat = projection(e, n, inverse=True)
    bearing, _, distance = pyproj.Geod(ellps="WGS84").inv(lon, lat, end_lon, end_lat)
    close(bearing, 0, 0.001, name + " true-north geodesic bearing")
    close(distance, 1000, 0.1, name + " ground distance")
    wall, = f.by_type("IfcWall")
    settings = ifcopenshell.geom.settings()
    settings.set("use-world-coords", True)
    shape = ifcopenshell.geom.create_shape(settings, wall)
    vertices = ifcopenshell.util.shape.get_vertices(shape.geometry)
    for actual, expected in zip(vertices.min(axis=0), (0, -.05, 0)):
        close(actual, expected, 1e-6, name + " wall minimum after IFC placement in metres")
    for actual, expected in zip(vertices.max(axis=0), (10, .05, 3)):
        close(actual, expected, 1e-6, name + " wall maximum after IFC placement in metres")
    close(ifcopenshell.util.shape.get_volume(shape.geometry), 3, 1e-6, name + " wall volume m3")
    imported = run(cli, "run", ifc, "--cmd", "geolocate {}")
    imported_location = next(value for value in imported if value.get("located"))
    for key in ("latitude", "longitude", "true_north", "elevation"):
        close(imported_location[key], location[key], 1e-6, name + " IFC reopen " + key)
    return {"name": name, "convergence": factors.meridian_convergence,
            "rotation": math.degrees(rotation), "point_scale": scale,
            "true_north_bearing": bearing, "ground_metres": distance,
            "schema_errors": 0, "unique_global_ids": len(ids), "wall_volume_m3": 3}


def main():
    cli = str(Path(sys.argv[1]).resolve())
    out = Path(sys.argv[2]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    cases = json.loads((Path(__file__).parent / "georef_cases.json").read_text())
    results = [check(cli, out, case) for case in cases]
    report = {"ifcopenshell": ifcopenshell.version, "pyproj": pyproj.__version__,
              "proj": pyproj.proj_version_str, "cases": results}
    (out / "acceptance.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
