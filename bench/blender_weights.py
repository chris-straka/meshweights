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
from mathutils import Matrix
from mathutils.kdtree import KDTree

HEAT_SCALE = 10.0


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
    # Bone heat has fixed tolerances and finds no solution on a small
    # figure (compact figure, 0.98 m: nothing bound; at ten times the size, all but
    # 20 of 3726 vertices). Weights don't depend on size: bind at ten
    # times, then scale back.
    up = Matrix.Scale(HEAT_SCALE, 4)
    mw = obj.matrix_world.copy()
    obj.parent = None
    obj.matrix_world = up @ mw
    arm.matrix_world = up @ arm.matrix_world
    bpy.context.view_layer.update()
    bpy.ops.object.mode_set(mode="OBJECT")
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    arm.select_set(True)
    bpy.context.view_layer.objects.active = arm
    try:
        bpy.ops.object.parent_set(type="ARMATURE_AUTO")
    finally:
        arm.matrix_world = up.inverted() @ arm.matrix_world
        bpy.context.view_layer.update()
        if obj.parent is not arm:
            obj.matrix_world = mw


def envelope_heat(obj, arm):
    """Heat on a closed voxel remesh of the mesh, its weights moved back
    by nearest face (rigforge's auto_rig does the same when heat finds
    no solution on a mesh of stacked or self-crossing shells)."""
    env = obj.copy()
    env.data = obj.data.copy()
    env.modifiers.clear()
    env.vertex_groups.clear()
    bpy.context.scene.collection.objects.link(env)
    dims = max(obj.dimensions)
    rm = env.modifiers.new("Remesh", "REMESH")
    rm.mode = "VOXEL"
    rm.voxel_size = max(dims / 180.0, 1e-4)
    bpy.ops.object.select_all(action="DESELECT")
    bpy.context.view_layer.objects.active = env
    env.select_set(True)
    bpy.ops.object.modifier_apply(modifier=rm.name)
    try:
        heat(env, arm)
    except RuntimeError as e:
        print(f"blender_weights: {obj.name}: envelope heat raised ({str(e)[:120]})")
    obj.vertex_groups.clear()
    for g in env.vertex_groups:
        obj.vertex_groups.new(name=g.name)
    bpy.ops.object.select_all(action="DESELECT")
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    dt = obj.modifiers.new("Weights", "DATA_TRANSFER")
    dt.object = env
    dt.use_vert_data = True
    dt.data_types_verts = {"VGROUP_WEIGHTS"}
    dt.vert_mapping = "POLYINTERP_NEAREST"
    dt.layers_vgroup_select_src = "ALL"
    dt.layers_vgroup_select_dst = "NAME"
    dt.mix_mode = "REPLACE"
    bpy.ops.object.modifier_apply(modifier=dt.name)
    print(f"blender_weights: {obj.name}: heat found nothing on the mesh; heat on a {len(env.data.vertices)}-vertex remesh, moved back")
    bpy.data.objects.remove(env)
    return "heat (remesh)"


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
            if not any(any(e.group in {g.index for g in obj.vertex_groups if g.name in names} and e.weight > 1e-4 for e in v.groups) for v in obj.data.vertices):
                how = envelope_heat(obj, arm)
        filled = fill_unassigned(obj, names)
        print(f"blender_weights: {obj.name}: {how}, {len(obj.data.vertices)} verts, {filled} filled from neighbours")
    bpy.ops.export_scene.gltf(filepath=out, export_format="GLB", export_animations=False, export_skins=True, export_yup=True)
    print(f"blender_weights: wrote {out}")


main()
