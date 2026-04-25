//! `rpcs3-hle-cell-freetype2` — Rust port of `rpcs3/Emu/Cell/Modules/cell_FreeType2.cpp`.
//!
//! PS3 FreeType2 font rendering PRX HLE. In the C++ RPCS3 source the entire
//! module is a pile of `UNIMPLEMENTED_FUNC(cell_FreeType2); return CELL_OK;`
//! stubs — the real FreeType implementation lives off-console and this shim
//! only exists so that PRX lookups resolve. The port keeps the ENTRY_POINTS
//! table byte-exact (155 entries) for symbol-based telemetry; every dispatch
//! just bumps a counter and returns `CELL_OK`.
#![no_std]
extern crate alloc;

use rpcs3_emu::CellError;
use rpcs3_emu_types as rpcs3_emu;

pub const CELL_OK: u32 = 0;

/// PRX module entry names (REG_FUNC'd in cpp:938..1096) — 155 entries,
/// preserved in registration order for telemetry / reverse-engineering parity.
pub const ENTRY_POINTS: &[&str] = &[
    "cellFreeType2Ex",
    "FT_Activate_Size",
    "FT_Add_Default_Modules",
    "FT_Add_Module",
    "FT_Alloc",
    "FT_Angle_Diff",
    "FT_Atan2",
    "FT_Attach_File",
    "FT_Attach_Stream",
    "FT_Bitmap_Convert",
    "FT_Bitmap_Copy",
    "FT_Bitmap_Done",
    "FT_Bitmap_Embolden",
    "FT_Bitmap_New",
    "FTC_CMapCache_Lookup",
    "FTC_CMapCache_New",
    "FT_CeilFix",
    "FTC_ImageCache_Lookup",
    "FTC_ImageCache_New",
    "FTC_Manager_Done",
    "FTC_Manager_LookupFace",
    "FTC_Manager_LookupSize",
    "FTC_Manager_New",
    "FTC_Manager_RemoveFaceID",
    "FTC_Node_Unref",
    "FT_Cos",
    "FTC_SBitCache_Lookup",
    "FTC_SBitCache_New",
    "FT_DivFix",
    "FT_Done_Face",
    "FT_Done_FreeType",
    "FT_Done_Glyph",
    "FT_Done_Library",
    "FT_Done_Size",
    "FT_FloorFix",
    "FT_Free",
    "FT_Get_BDF_Charset_ID",
    "FT_Get_BDF_Property",
    "FT_Get_Char_Index",
    "FT_Get_Charmap_Index",
    "FT_Get_CMap_Language_ID",
    "FT_Get_First_Char",
    "FT_Get_Glyph",
    "FT_Get_Glyph_Name",
    "FT_Get_Kerning",
    "FT_Get_MM_Var",
    "FT_Get_Module",
    "FT_Get_Multi_Master",
    "FT_Get_Name_Index",
    "FT_Get_Next_Char",
    "FT_Get_PFR_Advance",
    "FT_Get_PFR_Kerning",
    "FT_Get_PFR_Metrics",
    "FT_Get_Postscript_Name",
    "FT_Get_PS_Font_Info",
    "FT_Get_PS_Font_Private",
    "FT_Get_Renderer",
    "FT_Get_Sfnt_Name",
    "FT_Get_Sfnt_Name_Count",
    "FT_Get_Sfnt_Table",
    "FT_Get_SubGlyph_Info",
    "FT_Get_Track_Kerning",
    "FT_Get_TrueType_Engine_Type",
    "FT_Get_WinFNT_Header",
    "FT_Get_X11_Font_Format",
    "FT_Glyph_Copy",
    "FT_Glyph_Get_CBox",
    "FT_GlyphSlot_Own_Bitmap",
    "FT_Glyph_Stroke",
    "FT_Glyph_StrokeBorder",
    "FT_Glyph_To_Bitmap",
    "FT_Glyph_Transform",
    "FT_Has_PS_Glyph_Names",
    "FT_Init_FreeType",
    "FT_Library_Version",
    "FT_List_Add",
    "FT_List_Finalize",
    "FT_List_Find",
    "FT_List_Insert",
    "FT_List_Iterate",
    "FT_List_Remove",
    "FT_List_Up",
    "FT_Load_Char",
    "FT_Load_Glyph",
    "FT_Load_Sfnt_Table",
    "FT_Matrix_Invert",
    "FT_Matrix_Multiply",
    "FT_MulDiv",
    "FT_MulFix",
    "FT_New_Face",
    "FT_New_Library",
    "FT_New_Memory",
    "FT_New_Memory_Face",
    "FT_New_Size",
    "FT_Open_Face",
    "FT_OpenType_Free",
    "FT_OpenType_Validate",
    "FT_Outline_Check",
    "FT_Outline_Copy",
    "FT_Outline_Decompose",
    "FT_Outline_Done",
    "FT_Outline_Embolden",
    "FT_Outline_Get_BBox",
    "FT_Outline_Get_Bitmap",
    "FT_Outline_Get_CBox",
    "FT_Outline_GetInsideBorder",
    "FT_Outline_Get_Orientation",
    "FT_Outline_GetOutsideBorder",
    "FT_Outline_New",
    "FT_Outline_Render",
    "FT_Outline_Reverse",
    "FT_Outline_Transform",
    "FT_Outline_Translate",
    "FT_Realloc",
    "FT_Remove_Module",
    "FT_Render_Glyph",
    "FT_Request_Size",
    "FT_RoundFix",
    "FT_Select_Charmap",
    "FT_Select_Size",
    "FT_Set_Charmap",
    "FT_Set_Char_Size",
    "FT_Set_Debug_Hook",
    "FT_Set_MM_Blend_Coordinates",
    "FT_Set_MM_Design_Coordinates",
    "FT_Set_Pixel_Sizes",
    "FT_Set_Renderer",
    "FT_Set_Transform",
    "FT_Set_Var_Blend_Coordinates",
    "FT_Set_Var_Design_Coordinates",
    "FT_Sfnt_Table_Info",
    "FT_Sin",
    "FT_Stream_OpenGzip",
    "FT_Stream_OpenLZW",
    "FT_Stroker_BeginSubPath",
    "FT_Stroker_ConicTo",
    "FT_Stroker_CubicTo",
    "FT_Stroker_Done",
    "FT_Stroker_EndSubPath",
    "FT_Stroker_Export",
    "FT_Stroker_ExportBorder",
    "FT_Stroker_GetBorderCounts",
    "FT_Stroker_GetCounts",
    "FT_Stroker_LineTo",
    "FT_Stroker_New",
    "FT_Stroker_ParseOutline",
    "FT_Stroker_Rewind",
    "FT_Stroker_Set",
    "FT_Tan",
    "FT_Vector_From_Polar",
    "FT_Vector_Length",
    "FT_Vector_Polarize",
    "FT_Vector_Rotate",
    "FT_Vector_Transform",
    "FT_Vector_Unit",
];

/// Pure-stub dispatch counters — one slot per entry, indexed by position in
/// `ENTRY_POINTS`. Counters let tests verify that a given HLE call funneled
/// through the expected symbol even though the body is `CELL_OK`.
pub struct FreeType2Hle {
    pub call_counts: [u64; 155],
}

impl FreeType2Hle {
    pub const fn new() -> Self {
        Self { call_counts: [0; 155] }
    }

    /// Dispatch an entry by index. Mirrors `UNIMPLEMENTED_FUNC(...); return CELL_OK;`.
    pub fn dispatch(&mut self, index: usize) -> Result<u32, CellError> {
        if index >= ENTRY_POINTS.len() {
            return Err(CellError(0x8001_0000));
        }
        self.call_counts[index] = self.call_counts[index].saturating_add(1);
        Ok(CELL_OK)
    }

    /// Dispatch by name — linear scan; mirrors PRX name-resolution.
    pub fn dispatch_by_name(&mut self, name: &str) -> Result<u32, CellError> {
        match ENTRY_POINTS.iter().position(|&e| e == name) {
            Some(i) => self.dispatch(i),
            None => Err(CellError(0x8001_0000)),
        }
    }
}

impl Default for FreeType2Hle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_count_matches_cpp() {
        assert_eq!(ENTRY_POINTS.len(), 155, "REG_FUNC count in cpp:938..1096");
    }

    #[test]
    fn first_entry_is_cellfreetype2ex() {
        assert_eq!(ENTRY_POINTS[0], "cellFreeType2Ex", "cpp:940");
    }

    #[test]
    fn last_entry_is_vector_unit() {
        assert_eq!(ENTRY_POINTS[154], "FT_Vector_Unit", "cpp:1095");
    }

    #[test]
    fn dispatch_returns_ok_and_bumps_counter() {
        let mut hle = FreeType2Hle::new();
        assert_eq!(hle.dispatch(0).unwrap(), CELL_OK);
        assert_eq!(hle.call_counts[0], 1);
        assert_eq!(hle.dispatch(0).unwrap(), CELL_OK);
        assert_eq!(hle.call_counts[0], 2);
    }

    #[test]
    fn dispatch_out_of_range() {
        let mut hle = FreeType2Hle::new();
        let err = hle.dispatch(155).unwrap_err();
        assert_eq!(err.0, 0x8001_0000);
    }

    #[test]
    fn dispatch_by_name_hits_correct_slot() {
        let mut hle = FreeType2Hle::new();
        hle.dispatch_by_name("FT_Init_FreeType").unwrap();
        let idx = ENTRY_POINTS.iter().position(|&e| e == "FT_Init_FreeType").unwrap();
        assert_eq!(hle.call_counts[idx], 1);
    }

    #[test]
    fn dispatch_by_name_unknown() {
        let mut hle = FreeType2Hle::new();
        let err = hle.dispatch_by_name("Not_A_Real_Entry").unwrap_err();
        assert_eq!(err.0, 0x8001_0000);
    }
}
