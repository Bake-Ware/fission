#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <GeomFill_Trihedron.hxx>
#include <BRepOffsetAPI_MakePipe.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <BRepOffset_MakeOffset.hxx>
#include <BRepAlgoAPI_Defeaturing.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepGProp.hxx>
#include <BRepGProp_Face.hxx>
#include <GProp_GProps.hxx>
#include <BRepLib.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepTools.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <Geom2d_Line.hxx>
#include <gp_Circ.hxx>
#include <gp_Cylinder.hxx>
#include <gp_Dir2d.hxx>
#include <gp_Pnt2d.hxx>
#include <TopTools_ListOfShape.hxx>
#include <gp_Pln.hxx>
#include <cstdlib>
#include <iostream>
#include "fission-occt/src/shim.hxx"
#include "fission-occt/src/shim.rs.h"

#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI_ProjectPointOnCurve.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Precision.hxx>
#include <Geom_Circle.hxx>
#include <Geom_SurfaceOfLinearExtrusion.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColStd_Array1OfReal.hxx>
#include <TColgp_Array1OfPnt.hxx>
#include <Geom_ConicalSurface.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <Geom_Ellipse.hxx>
#include <Geom_Line.hxx>
#include <Geom_Plane.hxx>
#include <Geom_SphericalSurface.hxx>
#include <Geom_ToroidalSurface.hxx>
#include <ShapeFix_Face.hxx>
#include <ShapeFix_Solid.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Shell.hxx>
#include <gp_Ax2.hxx>
#include <gp_Ax3.hxx>
#include <gp_GTrsf.hxx>
#include <vector>
#include <cmath>

#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <ShapeFix_Shape.hxx>
#include <Standard_Failure.hxx>

#include <cstdlib>
#include <stdexcept>
#include <string>

namespace fission {

// OCCT throws Standard_Failure, which isn't a std::exception, so cxx can't
// catch it. Convert everything to std::runtime_error.
template <class F> auto guarded(const char *what, F f) -> decltype(f()) {
  try {
    return f();
  } catch (const Standard_Failure &e) {
    std::string msg = e.GetMessageString() ? e.GetMessageString() : "";
    throw std::runtime_error(std::string(what) + ": " + e.DynamicType()->Name() + (msg.empty() ? "" : " (" + msg + ")"));
  } catch (const std::exception &) {
    throw;
  } catch (...) {
    throw std::runtime_error(std::string(what) + ": unknown OCCT exception");
  }
}

bool shape_is_valid(const TopoDS_Shape &shape) {
  try {
    BRepCheck_Analyzer analyzer(shape);
    return analyzer.IsValid();
  } catch (...) {
    return false;
  }
}

std::unique_ptr<TopoDS_Shape> shape_fix(const TopoDS_Shape &shape) {
  return guarded("shape fix", [&] {
    Handle(ShapeFix_Shape) fix = new ShapeFix_Shape(shape);
    fix->Perform();
    return std::make_unique<TopoDS_Shape>(fix->Shape());
  });
}

std::unique_ptr<TopoDS_Shape> boolean_op(const TopoDS_Shape &a, const TopoDS_Shape &b, int op, double fuzzy) {
  return guarded("boolean", [&] {
    auto run = [&](BRepAlgoAPI_BooleanOperation &algo) {
      algo.SetRunParallel(false);
      if (std::getenv("FISSION_BOP_OBB")) algo.SetUseOBB(true);
      if (std::getenv("FISSION_BOP_NONDESTRUCTIVE")) algo.SetNonDestructive(true);
      if (std::getenv("FISSION_BOP_NOCHECK")) algo.SetCheckInverted(false);
      if (fuzzy > 0.0) {
        algo.SetFuzzyValue(fuzzy);
      }
      algo.Build();
      if (!algo.IsDone() || algo.HasErrors()) {
        throw std::runtime_error("boolean failed");
      }
      return std::make_unique<TopoDS_Shape>(algo.Shape());
    };
    switch (op) {
    case 0: {
      BRepAlgoAPI_Fuse algo;
      TopTools_ListOfShape args, tools;
      args.Append(a);
      tools.Append(b);
      algo.SetArguments(args);
      algo.SetTools(tools);
      return run(algo);
    }
    case 1: {
      BRepAlgoAPI_Cut algo;
      TopTools_ListOfShape args, tools;
      args.Append(a);
      tools.Append(b);
      algo.SetArguments(args);
      algo.SetTools(tools);
      return run(algo);
    }
    default: {
      BRepAlgoAPI_Common algo;
      TopTools_ListOfShape args, tools;
      args.Append(a);
      tools.Append(b);
      algo.SetArguments(args);
      algo.SetTools(tools);
      return run(algo);
    }
    }
  });
}

namespace {
gp_Pnt pnt(const std::array<double, 3> &v) { return gp_Pnt(v[0], v[1], v[2]); }
gp_Vec vec(const std::array<double, 3> &v) { return gp_Vec(v[0], v[1], v[2]); }
gp_Dir dir(const std::array<double, 3> &v) { return gp_Dir(v[0], v[1], v[2]); }

// An x direction perpendicular to `n`, preferring `hint`.
gp_Dir xdir(const gp_Dir &n, const std::array<double, 3> &hint) {
  gp_Vec h = vec(hint);
  gp_Vec nv(n);
  gp_Vec x = h - nv * h.Dot(nv);
  if (x.Magnitude() < 1e-12) {
    x = gp_Vec(n.IsParallel(gp::DX(), 1e-6) ? gp::DY() : gp::DX());
    x = x - nv * x.Dot(nv);
  }
  return gp_Dir(x);
}

Handle(Geom_BSplineCurve) make_bspline(const AsmSpline &s) {
  int n = int(s.poles.size() / 3);
  if (n < 2 || s.knots.size() < 2 || s.knots.size() != s.mults.size()) return nullptr;
  TColgp_Array1OfPnt poles(1, n);
  for (int i = 0; i < n; ++i) poles.SetValue(i + 1, gp_Pnt(s.poles[3 * i], s.poles[3 * i + 1], s.poles[3 * i + 2]));
  int k = int(s.knots.size());
  TColStd_Array1OfReal knots(1, k);
  TColStd_Array1OfInteger mults(1, k);
  for (int i = 0; i < k; ++i) {
    knots.SetValue(i + 1, s.knots[i]);
    mults.SetValue(i + 1, s.mults[i]);
  }
  if (int(s.weights.size()) == n) {
    TColStd_Array1OfReal weights(1, n);
    for (int i = 0; i < n; ++i) weights.SetValue(i + 1, s.weights[i]);
    return new Geom_BSplineCurve(poles, weights, knots, mults, s.degree);
  }
  return new Geom_BSplineCurve(poles, knots, mults, s.degree);
}

Handle(Geom_Curve) make_curve(const AsmEdge &e) {
  if (e.kind == 2) {
    return make_bspline(e.spline);
  }
  if (e.kind == 0) {
    return new Geom_Line(pnt(e.a), dir(e.b));
  }
  if (e.kind == 1) {
    gp_Dir n = dir(e.b);
    double major = vec(e.c).Magnitude();
    gp_Ax2 ax(pnt(e.a), n, xdir(n, e.c));
    if (std::abs(e.ratio - 1.0) < 1e-9) {
      return new Geom_Circle(ax, major);
    }
    return new Geom_Ellipse(ax, major, major * e.ratio);
  }
  return nullptr;
}

Handle(Geom_Surface) make_surface(const AsmFace &f) {
  switch (f.kind) {
  case 0: {
    gp_Dir n = dir(f.n);
    return new Geom_Plane(gp_Ax3(pnt(f.p), n, xdir(n, f.u)));
  }
  case 1: {
    gp_Dir axis = dir(f.n);
    double radius = vec(f.u).Magnitude();
    gp_Ax3 ax(pnt(f.p), axis, xdir(axis, f.u));
    if (std::abs(f.sin) < 1e-12) {
      return new Geom_CylindricalSurface(ax, radius);
    }
    return new Geom_ConicalSurface(ax, std::atan2(f.sin, f.cos), radius);
  }
  case 2: {
    gp_Dir pole = dir(f.n);
    return new Geom_SphericalSurface(gp_Ax3(pnt(f.p), pole, xdir(pole, f.u)), f.r1);
  }
  case 3: {
    gp_Dir n = dir(f.n);
    return new Geom_ToroidalSurface(gp_Ax3(pnt(f.p), n, xdir(n, f.u)), f.r1, f.r2);
  }
  case 4: {
    Handle(Geom_BSplineCurve) profile = make_bspline(f.spline);
    if (profile.IsNull()) return nullptr;
    return new Geom_SurfaceOfLinearExtrusion(profile, dir(f.n));
  }
  default:
    return nullptr;
  }
}
} // namespace

static double volume_of(const TopoDS_Shape &shape) {
  GProp_GProps props;
  BRepGProp::VolumeProperties(shape, props);
  return props.Mass();
}

std::unique_ptr<TopoDS_Shape> build_asm_body(const AsmBody &body, BuildStats &stats) {
  return guarded("f3d import", [&] {
    std::vector<TopoDS_Vertex> verts;
    for (size_t i = 0; i + 2 < body.vertices.size(); i += 3) {
      verts.push_back(BRepBuilderAPI_MakeVertex(gp_Pnt(body.vertices[i], body.vertices[i + 1], body.vertices[i + 2])));
    }
    // Edges are shared by their faces, so adjacent faces connect directly.
    std::vector<TopoDS_Edge> edges(body.edges.size());
    std::vector<bool> edge_ok(body.edges.size(), false);
    for (size_t i = 0; i < body.edges.size(); ++i) {
      const AsmEdge &e = body.edges[i];
      try {
        Handle(Geom_Curve) c = make_curve(e);
        if (c.IsNull() || e.start >= verts.size() || e.end >= verts.size()) continue;
        if (e.reversed) c = c->Reversed();
        const TopoDS_Vertex &v1 = verts[e.start];
        const TopoDS_Vertex &v2 = verts[e.end];
        BRepBuilderAPI_MakeEdge me;
        if (e.start == e.end) {
          // Closed edge (a full circle): one turn starting at the vertex.
          if (c->IsPeriodic()) {
            GeomAPI_ProjectPointOnCurve proj(BRep_Tool::Pnt(v1), c);
            if (proj.NbPoints() == 0) continue;
            double t0 = proj.LowerDistanceParameter();
            me = BRepBuilderAPI_MakeEdge(c, v1, v1, t0, t0 + c->Period());
          } else {
            // A closed spline: its whole parameter range.
            me = BRepBuilderAPI_MakeEdge(c, v1, v1, c->FirstParameter(), c->LastParameter());
          }
        } else {
          me = BRepBuilderAPI_MakeEdge(c, v1, v2);
        }
        if (me.IsDone()) {
          edges[i] = me.Edge();
          edge_ok[i] = true;
        }
      } catch (...) {
      }
    }

    BRepBuilderAPI_Sewing sewing(1e-3);
    for (const AsmFace &f : body.faces) {
      try {
        Handle(Geom_Surface) surf = make_surface(f);
        if (!surf.IsNull() && f.loops.empty()) {
          // No boundary: the whole closed surface (a full torus or sphere).
          BRepBuilderAPI_MakeFace whole(surf, Precision::Confusion());
          if (whole.IsDone()) {
            TopoDS_Face face = whole.Face();
            if (f.reversed) face.Reverse();
            sewing.Add(face);
            stats.faces_built++;
          } else {
            stats.faces_failed++;
          }
          continue;
        }
        bool ok = !surf.IsNull() && !f.loops.empty();
        std::vector<TopoDS_Wire> wires;
        for (const AsmLoop &lp : f.loops) {
          if (!ok) break;
          BRepBuilderAPI_MakeWire mw;
          for (const AsmCoedge &ce : lp.coedges) {
            if (ce.edge >= edges.size() || !edge_ok[ce.edge]) {
              ok = false;
              break;
            }
            TopoDS_Edge edge = edges[ce.edge];
            mw.Add(ce.reversed ? TopoDS::Edge(edge.Reversed()) : edge);
          }
          if (!ok || !mw.IsDone()) {
            ok = false;
            break;
          }
          wires.push_back(mw.Wire());
        }
        if (!ok) {
          if (std::getenv("FISSION_F3D_TRACE")) std::cerr << "face kind " << int(f.kind) << ": wire failed\n";
          stats.faces_failed++;
          continue;
        }
        BRepBuilderAPI_MakeFace mf(surf, wires[0], true);
        for (size_t w = 1; w < wires.size(); ++w) mf.Add(wires[w]);
        if (!mf.IsDone()) {
          if (std::getenv("FISSION_F3D_TRACE")) std::cerr << "face kind " << int(f.kind) << ": MakeFace failed\n";
          stats.faces_failed++;
          continue;
        }
        // Adds missing pcurves and sorts out which wire is outer.
        Handle(ShapeFix_Face) fix = new ShapeFix_Face(mf.Face());
        fix->Perform();
        TopoDS_Face face = fix->Face();
        if (f.reversed) face.Reverse();
        sewing.Add(face);
        stats.faces_built++;
      } catch (...) {
        stats.faces_failed++;
      }
    }
    if (stats.faces_built == 0) {
      // Nothing to sew (and a null shape has no type to ask about).
      return std::make_unique<TopoDS_Shape>();
    }
    sewing.Perform();
    TopoDS_Shape shape = sewing.SewedShape();
    if (shape.IsNull()) return std::make_unique<TopoDS_Shape>();

    // Closed shells become solids; a shell inside another is a void in it
    // (a shelled part has an outer skin and an inner one).
    TopoDS_Shape result = shape;
    stats.solid = false;
    std::vector<TopoDS_Shell> closed;
    std::vector<TopoDS_Shape> open;
    for (TopExp_Explorer ex(shape, TopAbs_SHELL); ex.More(); ex.Next()) {
      TopoDS_Shell sh = TopoDS::Shell(ex.Current());
      if (BRep_Tool::IsClosed(sh)) closed.push_back(sh);
      else open.push_back(sh);
    }
    if (!closed.empty()) {
      auto solid_of = [](const TopoDS_Shell &sh) {
        BRepBuilderAPI_MakeSolid ms(sh);
        Handle(ShapeFix_Solid) fs = new ShapeFix_Solid(ms.Solid());
        fs->Perform();
        return TopoDS::Solid(fs->Solid());
      };
      // Biggest first, so a void finds the skin around it.
      std::vector<std::pair<double, TopoDS_Shell>> by_size;
      for (const auto &sh : closed) by_size.push_back({std::abs(volume_of(solid_of(sh))), sh});
      std::sort(by_size.begin(), by_size.end(), [](const auto &a, const auto &b) { return a.first > b.first; });
      std::vector<TopoDS_Solid> solids;
      std::vector<std::vector<TopoDS_Shell>> voids;
      for (const auto &[size, sh] : by_size) {
        TopExp_Explorer v(sh, TopAbs_VERTEX);
        gp_Pnt probe = BRep_Tool::Pnt(TopoDS::Vertex(v.Current()));
        bool placed = false;
        for (size_t k = 0; k < solids.size() && !placed; ++k) {
          BRepClass3d_SolidClassifier inside(solids[k], probe, 1e-6);
          if (inside.State() == TopAbs_IN) {
            voids[k].push_back(sh);
            placed = true;
          }
        }
        if (!placed) {
          solids.push_back(solid_of(sh));
          voids.push_back({});
        }
      }
      BRep_Builder builder;
      TopoDS_Compound all;
      builder.MakeCompound(all);
      for (size_t k = 0; k < solids.size(); ++k) {
        TopoDS_Solid solid = solids[k];
        if (!voids[k].empty()) {
          BRepBuilderAPI_MakeSolid ms;
          for (TopExp_Explorer e(solids[k], TopAbs_SHELL); e.More(); e.Next()) ms.Add(TopoDS::Shell(e.Current()));
          for (const auto &v : voids[k]) ms.Add(v);
          Handle(ShapeFix_Solid) fs = new ShapeFix_Solid(ms.Solid());
          fs->Perform();
          solid = TopoDS::Solid(fs->Solid());
        }
        builder.Add(all, solid);
      }
      for (const auto &o : open) builder.Add(all, o);
      result = all;
      if (solids.size() == 1 && open.empty()) {
        TopExp_Explorer only(all, TopAbs_SOLID);
        result = only.Current();
      }
      stats.solid = open.empty();
    }

    if (body.transform.size() == 13) {
      const auto &t = body.transform;
      gp_GTrsf g;
      g.SetVectorialPart(gp_Mat(t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7], t[8]) * t[12]);
      g.SetTranslationPart(gp_XYZ(t[9], t[10], t[11]));
      BRepBuilderAPI_GTransform gt(result, g, true);
      if (gt.IsDone()) result = gt.Shape();
    }
    return std::make_unique<TopoDS_Shape>(result);
  });
}

std::unique_ptr<TopoDS_Shape> draft_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces, double dx, double dy, double dz,
                                          double px, double py, double pz, double radians) {
  return guarded("draft", [&] {
    gp_Dir pull(dx, dy, dz);
    gp_Pln neutral(gp_Pnt(px, py, pz), pull);
    BRepOffsetAPI_DraftAngle draft(solid);
    int n = 0;
    for (TopExp_Explorer ex(faces, TopAbs_FACE); ex.More(); ex.Next()) {
      draft.Add(TopoDS::Face(ex.Current()), pull, radians, neutral);
      if (!draft.AddDone()) throw std::runtime_error("a face can't be drafted from this plane (is it square to the pull direction?)");
      n++;
    }
    if (n == 0) throw std::runtime_error("pick faces to draft");
    draft.Build();
    if (!draft.IsDone()) throw std::runtime_error("the drafted faces couldn't be rebuilt");
    return std::make_unique<TopoDS_Shape>(draft.Shape());
  });
}

// A face's outward normal at the middle of its parameter range, and its area.
static gp_Vec outward_normal(const TopoDS_Face &face) {
  double u0, u1, v0, v1;
  BRepTools::UVBounds(face, u0, u1, v0, v1);
  BRepGProp_Face prop(face);
  gp_Pnt p;
  gp_Vec n;
  prop.Normal((u0 + u1) / 2, (v0 + v1) / 2, p, n);
  return n.Magnitude() > 0 ? n.Normalized() : n;
}

static double area_of(const TopoDS_Shape &shape) {
  GProp_GProps props;
  BRepGProp::SurfaceProperties(shape, props);
  return props.Mass();
}

// Whether every face meeting this flat face is a wall square to it (a
// plane or cylinder standing on it), so pushing the face is just an extrude.
static bool only_walls_around(const TopoDS_Shape &solid, const TopoDS_Face &face, const gp_Vec &n) {
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(solid, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  for (TopExp_Explorer e(face, TopAbs_EDGE); e.More(); e.Next()) {
    const TopTools_ListOfShape *around = edge_faces.Seek(e.Current());
    if (!around) return false;
    for (const TopoDS_Shape &other : *around) {
      if (other.IsSame(face)) continue;
      BRepAdaptor_Surface s(TopoDS::Face(other));
      if (s.GetType() == GeomAbs_Plane) {
        if (std::abs(gp_Vec(s.Plane().Axis().Direction()).Dot(n)) > 1e-6) return false;
      } else if (s.GetType() == GeomAbs_Cylinder) {
        if (gp_Vec(s.Cylinder().Axis().Direction()).Crossed(n).Magnitude() > 1e-6) return false;
      } else {
        return false;
      }
    }
  }
  return true;
}

static bool has_freeform_faces(const TopoDS_Shape &solid) {
  for (TopExp_Explorer ex(solid, TopAbs_FACE); ex.More(); ex.Next()) {
    switch (BRepAdaptor_Surface(TopoDS::Face(ex.Current())).GetType()) {
    case GeomAbs_Plane:
    case GeomAbs_Cylinder:
    case GeomAbs_Cone:
    case GeomAbs_Sphere:
    case GeomAbs_Torus:
      break;
    default:
      return true;
    }
  }
  return false;
}

// Push flat faces along their normals as extrusions joined to (or cut from)
// the solid.
static TopoDS_Shape extrude_faces(const TopoDS_Shape &solid, const std::vector<TopoDS_Face> &picked, double distance) {
  TopoDS_Shape result = solid;
  for (const TopoDS_Face &f : picked) {
    TopoDS_Shape prism = BRepPrimAPI_MakePrism(f, outward_normal(f) * distance).Shape();
    result = *boolean_op(result, prism, distance > 0 ? 0 : 1, 1e-6);
  }
  return result;
}

std::unique_ptr<TopoDS_Shape> offset_faces_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces, double distance) {
  return guarded("offset face", [&] {
    std::vector<TopoDS_Face> picked;
    for (TopExp_Explorer ex(faces, TopAbs_FACE); ex.More(); ex.Next()) picked.push_back(TopoDS::Face(ex.Current()));
    if (picked.empty()) throw std::runtime_error("pick faces to offset");
    bool flat = true, walls = true;
    for (const TopoDS_Face &f : picked) {
      flat = flat && BRepAdaptor_Surface(f).GetType() == GeomAbs_Plane;
      walls = walls && flat && only_walls_around(solid, f, outward_normal(f));
    }
    // Exact and quick when the faces around are walls. OCCT's offset can
    // also take tens of seconds on freeform faces (a thread's), so flat faces
    // there are extruded too.
    if (walls || (flat && has_freeform_faces(solid))) return std::make_unique<TopoDS_Shape>(extrude_faces(solid, picked, distance));

    // Otherwise every face offsets by 0 except the picked ones, and the
    // neighbours are re-intersected to meet them. OCCT sometimes returns a
    // wrong solid without complaint, so the volume change must be about the
    // faces' area times the distance (the neighbours add or take some).
    double before = volume_of(solid), swept = 0;
    for (const TopoDS_Face &f : picked) swept += area_of(f) * distance;
    try {
      BRepOffset_MakeOffset offset;
      offset.Initialize(solid, 0.0, 1e-6, BRepOffset_Skin, Standard_False, Standard_False, GeomAbs_Intersection);
      for (const TopoDS_Face &f : picked) offset.SetOffsetOnFace(f, distance);
      offset.MakeOffsetShape();
      if (offset.IsDone()) {
        double ratio = (volume_of(offset.Shape()) - before) / swept;
        if (ratio > 0.5 && ratio < 3.0) return std::make_unique<TopoDS_Shape>(offset.Shape());
      }
    } catch (const Standard_Failure &) {
    }
    if (flat) return std::make_unique<TopoDS_Shape>(extrude_faces(solid, picked, distance));
    throw std::runtime_error("the faces couldn't be moved cleanly that far");
  });
}

static TopoDS_Shape defeature(const TopoDS_Shape &solid, const TopTools_ListOfShape &remove) {
  BRepAlgoAPI_Defeaturing algo;
  algo.SetShape(solid);
  algo.AddFacesToRemove(remove);
  algo.SetRunParallel(Standard_False);
  algo.SetToFillHistory(Standard_False);
  algo.Build();
  if (!algo.IsDone() || algo.HasErrors()) return TopoDS_Shape();
  return algo.Shape();
}

// The curved faces joined smoothly (tangent) to `seed`, seed included: a
// chain of fillets and the blends at their corners.
static TopTools_ListOfShape tangent_chain(const TopoDS_Shape &solid, const TopTools_ListOfShape &seed) {
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(solid, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  TopTools_IndexedMapOfShape chain;
  for (const TopoDS_Shape &f : seed) chain.Add(f);
  for (int i = 1; i <= chain.Extent(); i++) {
    TopoDS_Face face = TopoDS::Face(chain(i));
    for (TopExp_Explorer e(face, TopAbs_EDGE); e.More(); e.Next()) {
      const TopTools_ListOfShape *around = edge_faces.Seek(e.Current());
      if (!around) continue;
      for (const TopoDS_Shape &other : *around) {
        if (other.IsSame(face) || chain.Contains(other)) continue;
        TopoDS_Face of = TopoDS::Face(other);
        if (BRepAdaptor_Surface(of).GetType() == GeomAbs_Plane) continue;
        if (BRep_Tool::Continuity(TopoDS::Edge(e.Current()), face, of) >= GeomAbs_G1) chain.Add(of);
      }
    }
  }
  TopTools_ListOfShape out;
  for (int i = 1; i <= chain.Extent(); i++) out.Append(chain(i));
  return out;
}

std::unique_ptr<TopoDS_Shape> remove_faces_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces) {
  return guarded("delete face", [&] {
    // Defeaturing: the neighbouring faces grow to close the gap.
    TopTools_ListOfShape remove;
    for (TopExp_Explorer ex(faces, TopAbs_FACE); ex.More(); ex.Next()) remove.Append(ex.Current());
    if (remove.IsEmpty()) throw std::runtime_error("pick faces to delete");
    double before = volume_of(solid);
    int faces_before = 0;
    for (TopExp_Explorer ex(solid, TopAbs_FACE); ex.More(); ex.Next()) faces_before++;
    // OCCT hands the solid back unchanged, without an error, when it can't
    // close the gap.
    auto changed = [&](const TopoDS_Shape &s) {
      if (s.IsNull()) return false;
      int n = 0;
      for (TopExp_Explorer ex(s, TopAbs_FACE); ex.More(); ex.Next()) n++;
      return n != faces_before || std::abs(volume_of(s) - before) > 1e-9 * std::max(1.0, std::abs(before));
    };
    TopoDS_Shape result = defeature(solid, remove);
    if (changed(result)) return std::make_unique<TopoDS_Shape>(result);
    // One fillet of a rounded chain can't go alone: take the whole chain
    // (for curved picks only; a flat face's tangent neighbours aren't it).
    bool curved = true;
    for (const TopoDS_Shape &f : remove) curved = curved && BRepAdaptor_Surface(TopoDS::Face(f)).GetType() != GeomAbs_Plane;
    TopTools_ListOfShape chain = curved ? tangent_chain(solid, remove) : remove;
    if (chain.Extent() > remove.Extent()) {
      result = defeature(solid, chain);
      if (changed(result)) return std::make_unique<TopoDS_Shape>(result);
    }
    throw std::runtime_error("the faces around these can't grow to close the gap (try selecting the whole feature)");
  });
}

// A profile swept along a helix of `radius` round `ax` (starting on its X
// direction), `turns` times, rising `pitch` per turn. The profile lies in the
// plane through the axis: (r, z) pairs, r outward from the helix and z along
// the axis, or a circle of `circle_r` when there are none. Binormal mode keeps
// it facing the axis all the way, as a thread or spring's section does.
static TopoDS_Shape helix_solid(const gp_Ax3 &ax, double radius, double pitch, double turns, bool left,
                                const std::vector<std::pair<double, double>> &section, double circle_r) {
  if (radius <= 0 || pitch <= 0 || turns <= 0) throw std::runtime_error("the helix needs a positive radius, pitch and turns");
  Handle(Geom_CylindricalSurface) cyl = new Geom_CylindricalSurface(ax, radius);
  double du = left ? -2 * M_PI : 2 * M_PI;
  Handle(Geom2d_Line) line = new Geom2d_Line(gp_Pnt2d(0, 0), gp_Dir2d(du, pitch));
  double length = turns * std::sqrt(du * du + pitch * pitch);
  TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(line, cyl, 0.0, length);
  BRepLib::BuildCurves3d(edge, 1e-6, GeomAbs_C1, 14, int(turns * 16) + 32);
  TopoDS_Wire spine = BRepBuilderAPI_MakeWire(edge);

  gp_Dir x = ax.XDirection(), z = ax.Direction();
  gp_Pnt start = ax.Location().Translated(gp_Vec(x) * radius);
  TopoDS_Wire profile;
  if (section.empty()) {
    gp_Circ circle(gp_Ax2(start, ax.YDirection(), x), circle_r);
    profile = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(circle));
  } else {
    BRepBuilderAPI_MakePolygon poly;
    for (auto [r, h] : section) poly.Add(start.Translated(gp_Vec(x) * r + gp_Vec(z) * h));
    poly.Close();
    profile = poly.Wire();
  }
  BRepOffsetAPI_MakePipeShell pipe(spine);
  pipe.SetMode(z);
  pipe.Add(profile, Standard_False, Standard_False);
  pipe.Build();
  if (!pipe.IsDone()) throw std::runtime_error("the section couldn't follow the helix");
  if (!pipe.MakeSolid()) throw std::runtime_error("the helical sweep didn't close into a solid");
  return pipe.Shape();
}

std::unique_ptr<TopoDS_Shape> coil_shape(double ox, double oy, double oz, double dx, double dy, double dz, double xx, double xy,
                                         double xz, double radius, double pitch, double turns, bool left, double section) {
  return guarded("coil", [&] {
    if (section <= 0) throw std::runtime_error("the section needs a positive size");
    if (section >= pitch) throw std::runtime_error("the section is thicker than the pitch, so the turns would overlap");
    if (section / 2 >= radius) throw std::runtime_error("the section is wider than the coil's radius");
    gp_Ax3 ax(gp_Pnt(ox, oy, oz), gp_Dir(dx, dy, dz), gp_Dir(xx, xy, xz));
    return std::make_unique<TopoDS_Shape>(helix_solid(ax, radius, pitch, turns, left, {}, section / 2));
  });
}

std::unique_ptr<TopoDS_Shape> thread_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces, double pitch, bool left) {
  return guarded("thread", [&] {
    TopExp_Explorer ex(faces, TopAbs_FACE);
    if (!ex.More()) throw std::runtime_error("pick a cylindrical face to thread");
    TopoDS_Face face = TopoDS::Face(ex.Current());
    BRepAdaptor_Surface surface(face);
    if (surface.GetType() != GeomAbs_Cylinder) throw std::runtime_error("threads go on cylindrical faces");
    gp_Cylinder cylinder = surface.Cylinder();
    gp_Ax3 ax = cylinder.Position();
    double radius = cylinder.Radius();
    double u0, u1, v0, v1;
    BRepTools::UVBounds(face, u0, u1, v0, v1);
    if (pitch <= 0 || pitch > (v1 - v0)) throw std::runtime_error("the pitch must be positive and fit on the face");

    // A hole's wall faces the axis; a shaft's faces away.
    BRepGProp_Face prop(face);
    gp_Pnt p;
    gp_Vec n;
    prop.Normal((u0 + u1) / 2, (v0 + v1) / 2, p, n);
    gp_Vec z(ax.Direction()), radial(ax.Location(), p);
    radial -= z * radial.Dot(z);
    bool internal = n.Dot(radial) < 0;

    // ISO metric 60° V: the groove cut between teeth, from a little outside
    // the surface down to the thread's root.
    double depth = (internal ? 0.5413 : 0.6134) * pitch, clear = 0.1 * pitch;
    double half = (depth + clear) * std::tan(M_PI / 6);
    double out = internal ? -1.0 : 1.0;
    std::vector<std::pair<double, double>> groove = {{out * clear, -half}, {out * clear, half}, {-out * depth, 0.0}};

    // Run past each end, then trim to the face so neighbours aren't cut.
    gp_Ax2 base(ax.Location().Translated(z * v0), ax.Direction(), ax.XDirection());
    TopoDS_Shape limit = BRepPrimAPI_MakeCylinder(base, radius + depth + 2 * clear, v1 - v0).Shape();
    double before = volume_of(solid);
    // OCCT's booleans fail on some helix phases (an empty or partial result
    // with no error), so check what was removed and retry a few variants.
    for (double lead : {1.0, 0.75, 0.5}) {
      gp_Ax3 start(ax.Location().Translated(z * (v0 - lead * pitch)), ax.Direction(), ax.XDirection());
      TopoDS_Shape cutter = helix_solid(start, radius, pitch, (v1 - v0) / pitch + 1 + lead, left, groove, 0);
      BRepAlgoAPI_Common trim(cutter, limit);
      if (!trim.IsDone() || trim.HasErrors()) continue;
      double groove_volume = volume_of(trim.Shape());
      for (double fuzzy : {1e-5, 0.0, 1e-4}) {
        std::unique_ptr<TopoDS_Shape> cut;
        try {
          cut = boolean_op(solid, trim.Shape(), 1, fuzzy);
        } catch (const std::exception &) {
          continue;
        }
        double removed = before - volume_of(*cut);
        if (removed > 1e-6 * before && removed <= groove_volume * 1.001 + 1e-9) return cut;
      }
    }
    throw std::runtime_error("the thread couldn't be cut cleanly (try a slightly different length or pitch)");
  });
}

std::unique_ptr<TopoDS_Shape> loft_shape(const TopoDS_Shape &sections, bool ruled) {
  return guarded("loft", [&] {
    BRepOffsetAPI_ThruSections loft(Standard_True, ruled ? Standard_True : Standard_False);
    int n = 0;
    for (TopExp_Explorer ex(sections, TopAbs_WIRE); ex.More(); ex.Next()) {
      loft.AddWire(TopoDS::Wire(ex.Current()));
      n++;
    }
    if (n < 2) throw std::runtime_error("a loft needs at least two profiles");
    loft.Build();
    if (!loft.IsDone()) throw std::runtime_error("the profiles couldn't be joined");
    return std::make_unique<TopoDS_Shape>(loft.Shape());
  });
}

std::unique_ptr<TopoDS_Shape> pipe_shape(const TopoDS_Shape &profile, const TopoDS_Shape &spine) {
  return guarded("sweep", [&] {
    if (spine.ShapeType() != TopAbs_WIRE) throw std::runtime_error("the path isn't a wire");
    const TopoDS_Wire &path = TopoDS::Wire(spine);
    auto sweep_wire = [&](const TopoDS_Wire &w) {
      BRepOffsetAPI_MakePipeShell pipe(path);
      pipe.SetMode(Standard_False); // corrected Frenet
      pipe.SetTransitionMode(BRepBuilderAPI_RightCorner);
      pipe.Add(w, Standard_False, Standard_True); // turn the profile square to the path
      pipe.Build();
      if (!pipe.IsDone()) throw std::runtime_error("the profile couldn't follow the path");
      if (!pipe.MakeSolid()) throw std::runtime_error("the sweep didn't close into a solid");
      return pipe.Shape();
    };
    TopoDS_Shape result;
    bool first = true;
    for (TopExp_Explorer ex(profile, TopAbs_WIRE); ex.More(); ex.Next()) {
      TopoDS_Shape piece = sweep_wire(TopoDS::Wire(ex.Current()));
      if (first) {
        result = piece;
        first = false;
      } else {
        BRepAlgoAPI_Cut cut(result, piece);
        if (!cut.IsDone()) throw std::runtime_error("couldn't cut a hole through the sweep");
        result = cut.Shape();
      }
    }
    if (first) throw std::runtime_error("the profile has no boundary");
    return std::make_unique<TopoDS_Shape>(result);
  });
}

size_t solid_count(const TopoDS_Shape &shape) {
  size_t n = 0;
  for (TopExp_Explorer ex(shape, TopAbs_SOLID); ex.More(); ex.Next()) n++;
  return n;
}

std::unique_ptr<TopoDS_Shape> solid_at(const TopoDS_Shape &shape, size_t index) {
  size_t n = 0;
  for (TopExp_Explorer ex(shape, TopAbs_SOLID); ex.More(); ex.Next()) {
    if (n++ == index) return std::make_unique<TopoDS_Shape>(ex.Current());
  }
  throw std::out_of_range("no such solid");
}

} // namespace fission
