# SPDX-License-Identifier: GPL-2.0-or-later
"""Rebind a rigged GLB's skin on its own skeleton with Blender's automatic
weights (bone heat) or rigforge's geodesic voxel weights, for scoring them
beside the route's weights (bench only; the weights go back onto the
original file with `examples/put_weights`).

    Blender --background --factory-startup --python bench/blender_weights.py -- \
        IN.glb OUT.glb --method heat|voxel [--rigforge PATH/TO/rigforge/rigforge]

Only DEF- bones deform when the skeleton has any. A skinned mesh that heat
leaves without weights (it can't solve every shell) gets the nearest
weighted vertex's weights, as rigforge's auto_rig does; a mesh that voxel
can't bind falls back to heat. Prints one line per mesh.
"""

import importlib.util
import os
import sys

import bpy
from mathutils.kdtree import KDTree


def args():
    a = sys.argv[sys.argv.index("--") + 1 :]
    src, out = a[0], a[1]
    method = a[a.index("--method") + 1] if "--method" in a else "heat"
    rf = a[a.index("--rigforge") + 1] if "--rigforge" in a else None
    return src, out, method, rf


def load_voxel(rf):
    spec = importlib.util.spec_from_file_location("rf_voxel", os.path.join(rf, "weights", "voxel.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def fill_unassigned(obj, names):
    vs = obj.data.vertices
    scope = {g.index for g in obj.vertex_groups if g.name in names}
    good = [v.index for v in vs if any(e.group in scope and e.weight > 1e-4 for e in v.groups)]
    bad = [v.index for v in vs if v.index not in set(good)]
    if not bad or not good:
        return len(bad) if not good else 0
    kd = KDTree(len(good))
    for i, vi in enumerate(good):
        kd.insert(vs[vi].co, i)
    kd.balance()
    for vi in bad:
        _, i, _ = kd.find(vs[vi].co)
        for e in vs[good[i]].groups:
            if e.group in scope and e.weight > 1e-4:
                obj.vertex_groups[e.group].add([vi], e.weight, "REPLACE")
    return len(bad)


def merge(obj):
    """Join the vertices the glTF importer split at UV seams and hard
    edges: bone heat finds no solution on a mesh in pieces (the
    weights go back by surface, so the split is not needed)."""
    import bmesh

    bm = bmesh.new()
    bm.from_mesh(obj.data)
    before = len(bm.verts)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    after = len(bm.verts)
    bm.to_mesh(obj.data)
    bm.free()
    return before, after


def heat(obj, arm):
    for m in [m for m in obj.modifiers if m.type == "ARMATURE"]:
        obj.modifiers.remove(m)
    obj.vertex_groups.clear()
    mw = obj.matrix_world.copy()
    obj.parent = None
    obj.matrix_world = mw
    bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    arm.select_set(True)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.parent_set(type="ARMATURE_AUTO")


def main():
    src, out, method, rf = args()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=src)
    arms = [o for o in bpy.data.objects if o.type == "ARMATURE"]
    if len(arms) != 1:
        print(f"blender_weights: {len(arms)} armatures; need one")
        sys.exit(1)
    arm = arms[0]
    bones = arm.data.bones
    if any(b.name.startswith("DEF-") for b in bones):
        for b in bones:
            b.use_deform = b.name.startswith("DEF-")
    names = {b.name for b in bones if b.use_deform}
    skinned = [o for o in bpy.data.objects if o.type == "MESH" and any(m.type == "ARMATURE" for m in o.modifiers) and len(o.vertex_groups)]
    vox = load_voxel(rf) if method == "voxel" else None
    for obj in skinned:
        before, after = merge(obj)
        print(f"blender_weights: {obj.name}: {before} -> {after} verts after joining seams")
        how = method
        if vox is not None:
            try:
                assignment, _ = vox.bind_weights(obj, arm, sorted(names))
                vox.apply_weights(obj, assignment)
                vox.ensure_armature_modifier(obj, arm)
            except Exception as e:  # a mesh voxel can't bind: heat
                print(f"blender_weights: {obj.name}: voxel failed ({str(e)[:120]}); heat instead")
                how = "heat"
        if how == "heat":
            try:
                heat(obj, arm)
            except RuntimeError as e:
                print(f"blender_weights: {obj.name}: heat raised ({str(e)[:120]})")
        filled = fill_unassigned(obj, names)
        print(f"blender_weights: {obj.name}: {how}, {len(obj.data.vertices)} verts, {filled} filled from neighbours")
    bpy.ops.export_scene.gltf(filepath=out, export_format="GLB", export_animations=False, export_skins=True, export_yup=True)
    print(f"blender_weights: wrote {out}")


main()
