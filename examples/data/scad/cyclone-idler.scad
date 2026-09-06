// Cyclone-PCB-Factory Y-axis smooth-rod idler.
// Distilled from carlosgs/Cyclone-PCB-Factory/Source_files/Cycl_Y_frames.scad
// (module Cyclone_Y_leftSmoothRodIdler) — inlined dependencies so the file
// compiles standalone. Original licence: CC BY-SA 4.0.
//
// A real-world CAD part: bracket with mounting foot, a rod-holder cylinder,
// a captive-nut screw pocket, and a debossed axis label ("L" / "R"). The
// text() at the end is what motivated maquette-scad's font support — it's
// how the printed part gets stamped so an assembler can tell left from
// right without cross-referencing the manual.

// ---- parameters -------------------------------------------------------
mirrorLogo         = false;   // false = "L" stamp, true = "R"
axes_Ysmooth_rodD  = 8;
footScrewSize      = 4;
rodScrewSize       = 3;
axes_Yreference_height = 20;
frame_footThickness    = 6;
holderThickness    = 5;

$fn = 48;

// ---- derived ----------------------------------------------------------
holderOuterRadius = holderThickness + axes_Ysmooth_rodD / 2;
dimX = holderOuterRadius * 2;
dimY = 5 + footScrewSize * 2;
dimZ = axes_Yreference_height;
slotHeight = 3;
screwLength = holderOuterRadius * 2;
footSeparation = footScrewSize * 2;

// ---- part -------------------------------------------------------------
difference() {
    union() {
        translate([0, 0, -axes_Yreference_height])
            cube([dimX, dimY, dimZ + holderThickness + axes_Ysmooth_rodD / 2]);
        translate([-holderOuterRadius, 0, -axes_Yreference_height])
            cube([dimX, dimY, dimZ]);
        rotate([-90, 0, 0])
            cylinder(r = holderOuterRadius, h = dimY);
        translate([0, dimY / 2, -axes_Yreference_height])
            hull() {
                translate([-holderOuterRadius - footSeparation, 0, 0])
                    cylinder(r = dimY / 2, h = frame_footThickness);
                translate([holderOuterRadius * 2 + footSeparation, 0, 0])
                    cylinder(r = dimY / 2, h = frame_footThickness);
                translate([holderOuterRadius / 2, dimY / 2 + footSeparation, 0])
                    cylinder(r = dimY / 2, h = frame_footThickness);
            }
    }
    // rod pass-through
    rotate([-90, 0, 0])
        translate([0, 0, -1])
            cylinder(d = axes_Ysmooth_rodD, h = dimY + 2);
    // clamp slot
    translate([dimX / 2, dimY / 2, 0])
        cube([dimX + 1, dimY + 1, slotHeight], center = true);
    // clamp screw
    translate([2.5 + holderOuterRadius, dimY / 2, holderOuterRadius])
        rotate([0, 90, 0])
            translate([0, 0, -screwLength / 2])
                cylinder(d = rodScrewSize + 0.3, h = screwLength + 10);
    // foot mounting holes
    for (p = [ [-holderOuterRadius - footSeparation, 0],
               [ holderOuterRadius * 2 + footSeparation, 0],
               [ holderOuterRadius / 2, dimY / 2 + footSeparation] ])
        translate([p[0], dimY / 2 + p[1], -axes_Yreference_height - 0.1])
            cylinder(d = footScrewSize + 0.3, h = frame_footThickness + 1);
    // axis label — the reason maquette-scad now needs a font engine.
    translate([dimX, dimY / 2, -axes_Yreference_height / 2])
        rotate([0, 0, 90 + (mirrorLogo ? 180 : 0)])
            rotate([90, 0, 0])
                linear_extrude(height = 2, center = true)
                    text(mirrorLogo ? "R" : "L",
                         size = 7.5, halign = "center", valign = "center");
}
