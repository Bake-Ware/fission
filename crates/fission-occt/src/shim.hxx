#pragma once
#include "rust/cxx.h"
#include <memory>
#include <TopoDS_Shape.hxx>

namespace fission {
struct AsmBody;
struct BuildStats;
std::unique_ptr<TopoDS_Shape> draft_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces, double dx, double dy, double dz,
                                          double px, double py, double pz, double radians);
std::unique_ptr<TopoDS_Shape> offset_faces_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces, double distance);
std::unique_ptr<TopoDS_Shape> remove_faces_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces);
std::unique_ptr<TopoDS_Shape> coil_shape(double ox, double oy, double oz, double dx, double dy, double dz, double xx, double xy,
                                         double xz, double radius, double pitch, double turns, bool left, double section);
std::unique_ptr<TopoDS_Shape> thread_shape(const TopoDS_Shape &solid, const TopoDS_Shape &faces, double pitch, bool left);
std::unique_ptr<TopoDS_Shape> loft_shape(const TopoDS_Shape &sections, bool ruled);
std::unique_ptr<TopoDS_Shape> pipe_shape(const TopoDS_Shape &profile, const TopoDS_Shape &spine);
size_t solid_count(const TopoDS_Shape &shape);
std::unique_ptr<TopoDS_Shape> solid_at(const TopoDS_Shape &shape, size_t index);
std::unique_ptr<TopoDS_Shape> build_asm_body(const AsmBody &body, BuildStats &stats);
bool shape_is_valid(const TopoDS_Shape &shape);
std::unique_ptr<TopoDS_Shape> shape_fix(const TopoDS_Shape &shape);
std::unique_ptr<TopoDS_Shape> boolean_op(const TopoDS_Shape &a, const TopoDS_Shape &b, int op, double fuzzy);
} // namespace fission
