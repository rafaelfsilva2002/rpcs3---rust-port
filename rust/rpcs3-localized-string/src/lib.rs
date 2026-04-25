//! `rpcs3-localized-string` — Rust port of
//! `rpcs3/Emu/localized_string.cpp` + `localized_string_id.h`.
//!
//! Type-safe integer wrapper around the 315 UI string IDs RPCS3 uses
//! across overlays, dialogs, home menu, trophies, recording, RPCN, and
//! progress screens. The cpp enum is `enum class localized_string_id`
//! with implicit sequential discriminants; we mirror that exactly.
//!
//! Frozen:
//!
//! - **315 total variants** (cpp header:3..end).
//! - `INVALID = 0`, `RSX_OVERLAYS_SPINNER_NO_TEXT = 1`, ...,
//!   `SAVESTATE_FAILED_DUE_TO_MISSING_SPU_SETTING = 314`.
//! - The underlying type is `u32` (cpp `enum class` defaults to `int`
//!   which round-trips through `u32` safely here).

/// Type-safe wrapper for a single localized string id. The inner `u32`
/// matches the cpp enum's discriminant.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalizedStringId(pub u32);

/// Total number of discriminants in the cpp enum (including INVALID at 0).
pub const LOCALIZED_STRING_ID_COUNT: u32 = 315;

pub const INVALID: LocalizedStringId = LocalizedStringId(0);
pub const RSX_OVERLAYS_SPINNER_NO_TEXT: LocalizedStringId = LocalizedStringId(1);
pub const RSX_OVERLAYS_TROPHY_BRONZE: LocalizedStringId = LocalizedStringId(2);
pub const RSX_OVERLAYS_TROPHY_SILVER: LocalizedStringId = LocalizedStringId(3);
pub const RSX_OVERLAYS_TROPHY_GOLD: LocalizedStringId = LocalizedStringId(4);
pub const RSX_OVERLAYS_TROPHY_PLATINUM: LocalizedStringId = LocalizedStringId(5);
pub const RSX_OVERLAYS_COMPILING_SHADERS: LocalizedStringId = LocalizedStringId(6);
pub const RSX_OVERLAYS_COMPILING_PPU_MODULES: LocalizedStringId = LocalizedStringId(7);
pub const RSX_OVERLAYS_MSG_DIALOG_YES: LocalizedStringId = LocalizedStringId(8);
pub const RSX_OVERLAYS_MSG_DIALOG_NO: LocalizedStringId = LocalizedStringId(9);
pub const RSX_OVERLAYS_MSG_DIALOG_CANCEL: LocalizedStringId = LocalizedStringId(10);
pub const RSX_OVERLAYS_MSG_DIALOG_OK: LocalizedStringId = LocalizedStringId(11);
pub const RSX_OVERLAYS_SAVE_DIALOG_TITLE: LocalizedStringId = LocalizedStringId(12);
pub const RSX_OVERLAYS_SAVE_DIALOG_DELETE: LocalizedStringId = LocalizedStringId(13);
pub const RSX_OVERLAYS_SAVE_DIALOG_LOAD: LocalizedStringId = LocalizedStringId(14);
pub const RSX_OVERLAYS_SAVE_DIALOG_SAVE: LocalizedStringId = LocalizedStringId(15);
pub const RSX_OVERLAYS_OSK_DIALOG_ACCEPT: LocalizedStringId = LocalizedStringId(16);
pub const RSX_OVERLAYS_OSK_DIALOG_CANCEL: LocalizedStringId = LocalizedStringId(17);
pub const RSX_OVERLAYS_OSK_DIALOG_SPACE: LocalizedStringId = LocalizedStringId(18);
pub const RSX_OVERLAYS_OSK_DIALOG_BACKSPACE: LocalizedStringId = LocalizedStringId(19);
pub const RSX_OVERLAYS_OSK_DIALOG_SHIFT: LocalizedStringId = LocalizedStringId(20);
pub const RSX_OVERLAYS_OSK_DIALOG_ENTER_TEXT: LocalizedStringId = LocalizedStringId(21);
pub const RSX_OVERLAYS_OSK_DIALOG_ENTER_PASSWORD: LocalizedStringId = LocalizedStringId(22);
pub const RSX_OVERLAYS_MEDIA_DIALOG_TITLE: LocalizedStringId = LocalizedStringId(23);
pub const RSX_OVERLAYS_MEDIA_DIALOG_TITLE_PHOTO_IMPORT: LocalizedStringId = LocalizedStringId(24);
pub const RSX_OVERLAYS_MEDIA_DIALOG_EMPTY: LocalizedStringId = LocalizedStringId(25);
pub const RSX_OVERLAYS_LIST_SELECT: LocalizedStringId = LocalizedStringId(26);
pub const RSX_OVERLAYS_LIST_CANCEL: LocalizedStringId = LocalizedStringId(27);
pub const RSX_OVERLAYS_LIST_DENY: LocalizedStringId = LocalizedStringId(28);
pub const RSX_OVERLAYS_PRESSURE_INTENSITY_TOGGLED_OFF: LocalizedStringId = LocalizedStringId(29);
pub const RSX_OVERLAYS_PRESSURE_INTENSITY_TOGGLED_ON: LocalizedStringId = LocalizedStringId(30);
pub const RSX_OVERLAYS_ANALOG_LIMITER_TOGGLED_OFF: LocalizedStringId = LocalizedStringId(31);
pub const RSX_OVERLAYS_ANALOG_LIMITER_TOGGLED_ON: LocalizedStringId = LocalizedStringId(32);
pub const RSX_OVERLAYS_MOUSE_AND_KEYBOARD_EMULATED: LocalizedStringId = LocalizedStringId(33);
pub const RSX_OVERLAYS_MOUSE_AND_KEYBOARD_PAD: LocalizedStringId = LocalizedStringId(34);
pub const CELL_GAME_ERROR_BROKEN_GAMEDATA: LocalizedStringId = LocalizedStringId(35);
pub const CELL_GAME_ERROR_BROKEN_HDDGAME: LocalizedStringId = LocalizedStringId(36);
pub const CELL_GAME_ERROR_BROKEN_EXIT_GAMEDATA: LocalizedStringId = LocalizedStringId(37);
pub const CELL_GAME_ERROR_BROKEN_EXIT_HDDGAME: LocalizedStringId = LocalizedStringId(38);
pub const CELL_GAME_ERROR_NOSPACE: LocalizedStringId = LocalizedStringId(39);
pub const CELL_GAME_ERROR_NOSPACE_EXIT: LocalizedStringId = LocalizedStringId(40);
pub const CELL_GAME_ERROR_DIR_NAME: LocalizedStringId = LocalizedStringId(41);
pub const CELL_GAME_DATA_EXIT_BROKEN: LocalizedStringId = LocalizedStringId(42);
pub const CELL_HDD_GAME_EXIT_BROKEN: LocalizedStringId = LocalizedStringId(43);
pub const CELL_HDD_GAME_CHECK_NOSPACE: LocalizedStringId = LocalizedStringId(44);
pub const CELL_HDD_GAME_CHECK_BROKEN: LocalizedStringId = LocalizedStringId(45);
pub const CELL_HDD_GAME_CHECK_NODATA: LocalizedStringId = LocalizedStringId(46);
pub const CELL_HDD_GAME_CHECK_INVALID: LocalizedStringId = LocalizedStringId(47);
pub const CELL_GAMEDATA_CHECK_NOSPACE: LocalizedStringId = LocalizedStringId(48);
pub const CELL_GAMEDATA_CHECK_BROKEN: LocalizedStringId = LocalizedStringId(49);
pub const CELL_GAMEDATA_CHECK_NODATA: LocalizedStringId = LocalizedStringId(50);
pub const CELL_GAMEDATA_CHECK_INVALID: LocalizedStringId = LocalizedStringId(51);
pub const CELL_MSG_DIALOG_ERROR_DEFAULT: LocalizedStringId = LocalizedStringId(52);
pub const CELL_MSG_DIALOG_ERROR_80010001: LocalizedStringId = LocalizedStringId(53);
pub const CELL_MSG_DIALOG_ERROR_80010002: LocalizedStringId = LocalizedStringId(54);
pub const CELL_MSG_DIALOG_ERROR_80010003: LocalizedStringId = LocalizedStringId(55);
pub const CELL_MSG_DIALOG_ERROR_80010004: LocalizedStringId = LocalizedStringId(56);
pub const CELL_MSG_DIALOG_ERROR_80010005: LocalizedStringId = LocalizedStringId(57);
pub const CELL_MSG_DIALOG_ERROR_80010006: LocalizedStringId = LocalizedStringId(58);
pub const CELL_MSG_DIALOG_ERROR_80010007: LocalizedStringId = LocalizedStringId(59);
pub const CELL_MSG_DIALOG_ERROR_80010008: LocalizedStringId = LocalizedStringId(60);
pub const CELL_MSG_DIALOG_ERROR_80010009: LocalizedStringId = LocalizedStringId(61);
pub const CELL_MSG_DIALOG_ERROR_8001000A: LocalizedStringId = LocalizedStringId(62);
pub const CELL_MSG_DIALOG_ERROR_8001000B: LocalizedStringId = LocalizedStringId(63);
pub const CELL_MSG_DIALOG_ERROR_8001000C: LocalizedStringId = LocalizedStringId(64);
pub const CELL_MSG_DIALOG_ERROR_8001000D: LocalizedStringId = LocalizedStringId(65);
pub const CELL_MSG_DIALOG_ERROR_8001000F: LocalizedStringId = LocalizedStringId(66);
pub const CELL_MSG_DIALOG_ERROR_80010010: LocalizedStringId = LocalizedStringId(67);
pub const CELL_MSG_DIALOG_ERROR_80010011: LocalizedStringId = LocalizedStringId(68);
pub const CELL_MSG_DIALOG_ERROR_80010012: LocalizedStringId = LocalizedStringId(69);
pub const CELL_MSG_DIALOG_ERROR_80010013: LocalizedStringId = LocalizedStringId(70);
pub const CELL_MSG_DIALOG_ERROR_80010014: LocalizedStringId = LocalizedStringId(71);
pub const CELL_MSG_DIALOG_ERROR_80010015: LocalizedStringId = LocalizedStringId(72);
pub const CELL_MSG_DIALOG_ERROR_80010016: LocalizedStringId = LocalizedStringId(73);
pub const CELL_MSG_DIALOG_ERROR_80010017: LocalizedStringId = LocalizedStringId(74);
pub const CELL_MSG_DIALOG_ERROR_80010018: LocalizedStringId = LocalizedStringId(75);
pub const CELL_MSG_DIALOG_ERROR_80010019: LocalizedStringId = LocalizedStringId(76);
pub const CELL_MSG_DIALOG_ERROR_8001001A: LocalizedStringId = LocalizedStringId(77);
pub const CELL_MSG_DIALOG_ERROR_8001001B: LocalizedStringId = LocalizedStringId(78);
pub const CELL_MSG_DIALOG_ERROR_8001001C: LocalizedStringId = LocalizedStringId(79);
pub const CELL_MSG_DIALOG_ERROR_8001001D: LocalizedStringId = LocalizedStringId(80);
pub const CELL_MSG_DIALOG_ERROR_8001001E: LocalizedStringId = LocalizedStringId(81);
pub const CELL_MSG_DIALOG_ERROR_8001001F: LocalizedStringId = LocalizedStringId(82);
pub const CELL_MSG_DIALOG_ERROR_80010020: LocalizedStringId = LocalizedStringId(83);
pub const CELL_MSG_DIALOG_ERROR_80010021: LocalizedStringId = LocalizedStringId(84);
pub const CELL_MSG_DIALOG_ERROR_80010022: LocalizedStringId = LocalizedStringId(85);
pub const CELL_MSG_DIALOG_ERROR_80010023: LocalizedStringId = LocalizedStringId(86);
pub const CELL_MSG_DIALOG_ERROR_80010024: LocalizedStringId = LocalizedStringId(87);
pub const CELL_MSG_DIALOG_ERROR_80010025: LocalizedStringId = LocalizedStringId(88);
pub const CELL_MSG_DIALOG_ERROR_80010026: LocalizedStringId = LocalizedStringId(89);
pub const CELL_MSG_DIALOG_ERROR_80010027: LocalizedStringId = LocalizedStringId(90);
pub const CELL_MSG_DIALOG_ERROR_80010028: LocalizedStringId = LocalizedStringId(91);
pub const CELL_MSG_DIALOG_ERROR_80010029: LocalizedStringId = LocalizedStringId(92);
pub const CELL_MSG_DIALOG_ERROR_8001002A: LocalizedStringId = LocalizedStringId(93);
pub const CELL_MSG_DIALOG_ERROR_8001002B: LocalizedStringId = LocalizedStringId(94);
pub const CELL_MSG_DIALOG_ERROR_8001002C: LocalizedStringId = LocalizedStringId(95);
pub const CELL_MSG_DIALOG_ERROR_8001002D: LocalizedStringId = LocalizedStringId(96);
pub const CELL_MSG_DIALOG_ERROR_8001002E: LocalizedStringId = LocalizedStringId(97);
pub const CELL_MSG_DIALOG_ERROR_8001002F: LocalizedStringId = LocalizedStringId(98);
pub const CELL_MSG_DIALOG_ERROR_80010030: LocalizedStringId = LocalizedStringId(99);
pub const CELL_MSG_DIALOG_ERROR_80010031: LocalizedStringId = LocalizedStringId(100);
pub const CELL_MSG_DIALOG_ERROR_80010032: LocalizedStringId = LocalizedStringId(101);
pub const CELL_MSG_DIALOG_ERROR_80010033: LocalizedStringId = LocalizedStringId(102);
pub const CELL_MSG_DIALOG_ERROR_80010034: LocalizedStringId = LocalizedStringId(103);
pub const CELL_MSG_DIALOG_ERROR_80010035: LocalizedStringId = LocalizedStringId(104);
pub const CELL_MSG_DIALOG_ERROR_80010036: LocalizedStringId = LocalizedStringId(105);
pub const CELL_MSG_DIALOG_ERROR_80010037: LocalizedStringId = LocalizedStringId(106);
pub const CELL_MSG_DIALOG_ERROR_80010038: LocalizedStringId = LocalizedStringId(107);
pub const CELL_MSG_DIALOG_ERROR_80010039: LocalizedStringId = LocalizedStringId(108);
pub const CELL_MSG_DIALOG_ERROR_8001003A: LocalizedStringId = LocalizedStringId(109);
pub const CELL_MSG_DIALOG_ERROR_8001003B: LocalizedStringId = LocalizedStringId(110);
pub const CELL_MSG_DIALOG_ERROR_8001003C: LocalizedStringId = LocalizedStringId(111);
pub const CELL_MSG_DIALOG_ERROR_8001003D: LocalizedStringId = LocalizedStringId(112);
pub const CELL_MSG_DIALOG_ERROR_8001003E: LocalizedStringId = LocalizedStringId(113);
pub const CELL_OSK_DIALOG_TITLE: LocalizedStringId = LocalizedStringId(114);
pub const CELL_OSK_DIALOG_BUSY: LocalizedStringId = LocalizedStringId(115);
pub const CELL_SAVEDATA_CB_BROKEN: LocalizedStringId = LocalizedStringId(116);
pub const CELL_SAVEDATA_CB_FAILURE: LocalizedStringId = LocalizedStringId(117);
pub const CELL_SAVEDATA_CB_NO_DATA: LocalizedStringId = LocalizedStringId(118);
pub const CELL_SAVEDATA_CB_NO_SPACE: LocalizedStringId = LocalizedStringId(119);
pub const CELL_SAVEDATA_NO_DATA: LocalizedStringId = LocalizedStringId(120);
pub const CELL_SAVEDATA_NEW_SAVED_DATA_TITLE: LocalizedStringId = LocalizedStringId(121);
pub const CELL_SAVEDATA_NEW_SAVED_DATA_SUB_TITLE: LocalizedStringId = LocalizedStringId(122);
pub const CELL_SAVEDATA_SAVE_CONFIRMATION: LocalizedStringId = LocalizedStringId(123);
pub const CELL_SAVEDATA_DELETE_CONFIRMATION: LocalizedStringId = LocalizedStringId(124);
pub const CELL_SAVEDATA_DELETE_SUCCESS: LocalizedStringId = LocalizedStringId(125);
pub const CELL_SAVEDATA_DELETE: LocalizedStringId = LocalizedStringId(126);
pub const CELL_SAVEDATA_SAVE: LocalizedStringId = LocalizedStringId(127);
pub const CELL_SAVEDATA_LOAD: LocalizedStringId = LocalizedStringId(128);
pub const CELL_SAVEDATA_OVERWRITE: LocalizedStringId = LocalizedStringId(129);
pub const CELL_SAVEDATA_AUTOSAVE: LocalizedStringId = LocalizedStringId(130);
pub const CELL_SAVEDATA_AUTOLOAD: LocalizedStringId = LocalizedStringId(131);
pub const CELL_CROSS_CONTROLLER_MSG: LocalizedStringId = LocalizedStringId(132);
pub const CELL_CROSS_CONTROLLER_FW_MSG: LocalizedStringId = LocalizedStringId(133);
pub const CELL_NP_RECVMESSAGE_DIALOG_TITLE: LocalizedStringId = LocalizedStringId(134);
pub const CELL_NP_RECVMESSAGE_DIALOG_TITLE_INVITE: LocalizedStringId = LocalizedStringId(135);
pub const CELL_NP_RECVMESSAGE_DIALOG_TITLE_ADD_FRIEND: LocalizedStringId = LocalizedStringId(136);
pub const CELL_NP_RECVMESSAGE_DIALOG_FROM: LocalizedStringId = LocalizedStringId(137);
pub const CELL_NP_RECVMESSAGE_DIALOG_SUBJECT: LocalizedStringId = LocalizedStringId(138);
pub const CELL_NP_SENDMESSAGE_DIALOG_TITLE: LocalizedStringId = LocalizedStringId(139);
pub const CELL_NP_SENDMESSAGE_DIALOG_TITLE_INVITE: LocalizedStringId = LocalizedStringId(140);
pub const CELL_NP_SENDMESSAGE_DIALOG_TITLE_ADD_FRIEND: LocalizedStringId = LocalizedStringId(141);
pub const CELL_NP_SENDMESSAGE_DIALOG_CONFIRMATION: LocalizedStringId = LocalizedStringId(142);
pub const CELL_NP_SENDMESSAGE_DIALOG_CONFIRMATION_INVITE: LocalizedStringId = LocalizedStringId(143);
pub const CELL_NP_SENDMESSAGE_DIALOG_CONFIRMATION_ADD_FRIEND: LocalizedStringId = LocalizedStringId(144);
pub const CELL_NP_MESSAGE_INVITE_RECEIVED: LocalizedStringId = LocalizedStringId(145);
pub const CELL_NP_MESSAGE_OTHER_RECEIVED: LocalizedStringId = LocalizedStringId(146);
pub const RECORDING_ABORTED: LocalizedStringId = LocalizedStringId(147);
pub const RPCN_NO_ERROR: LocalizedStringId = LocalizedStringId(148);
pub const RPCN_ERROR_INVALID_INPUT: LocalizedStringId = LocalizedStringId(149);
pub const RPCN_ERROR_WOLFSSL: LocalizedStringId = LocalizedStringId(150);
pub const RPCN_ERROR_RESOLVE: LocalizedStringId = LocalizedStringId(151);
pub const RPCN_ERROR_BINDING: LocalizedStringId = LocalizedStringId(152);
pub const RPCN_ERROR_CONNECT: LocalizedStringId = LocalizedStringId(153);
pub const RPCN_ERROR_LOGIN_ERROR: LocalizedStringId = LocalizedStringId(154);
pub const RPCN_ERROR_ALREADY_LOGGED: LocalizedStringId = LocalizedStringId(155);
pub const RPCN_ERROR_INVALID_LOGIN: LocalizedStringId = LocalizedStringId(156);
pub const RPCN_ERROR_INVALID_PASSWORD: LocalizedStringId = LocalizedStringId(157);
pub const RPCN_ERROR_INVALID_TOKEN: LocalizedStringId = LocalizedStringId(158);
pub const RPCN_ERROR_INVALID_PROTOCOL_VERSION: LocalizedStringId = LocalizedStringId(159);
pub const RPCN_ERROR_UNKNOWN: LocalizedStringId = LocalizedStringId(160);
pub const RPCN_SUCCESS_LOGGED_ON: LocalizedStringId = LocalizedStringId(161);
pub const RPCN_FRIEND_REQUEST_RECEIVED: LocalizedStringId = LocalizedStringId(162);
pub const RPCN_FRIEND_ADDED: LocalizedStringId = LocalizedStringId(163);
pub const RPCN_FRIEND_LOST: LocalizedStringId = LocalizedStringId(164);
pub const RPCN_FRIEND_LOGGED_IN: LocalizedStringId = LocalizedStringId(165);
pub const RPCN_FRIEND_LOGGED_OUT: LocalizedStringId = LocalizedStringId(166);
pub const HOME_MENU_TITLE: LocalizedStringId = LocalizedStringId(167);
pub const HOME_MENU_EXIT_GAME: LocalizedStringId = LocalizedStringId(168);
pub const HOME_MENU_RESTART: LocalizedStringId = LocalizedStringId(169);
pub const HOME_MENU_RESUME: LocalizedStringId = LocalizedStringId(170);
pub const HOME_MENU_FRIENDS: LocalizedStringId = LocalizedStringId(171);
pub const HOME_MENU_FRIENDS_REQUESTS: LocalizedStringId = LocalizedStringId(172);
pub const HOME_MENU_FRIENDS_BLOCKED: LocalizedStringId = LocalizedStringId(173);
pub const HOME_MENU_FRIENDS_STATUS_ONLINE: LocalizedStringId = LocalizedStringId(174);
pub const HOME_MENU_FRIENDS_STATUS_OFFLINE: LocalizedStringId = LocalizedStringId(175);
pub const HOME_MENU_FRIENDS_STATUS_BLOCKED: LocalizedStringId = LocalizedStringId(176);
pub const HOME_MENU_FRIENDS_REQUEST_SENT: LocalizedStringId = LocalizedStringId(177);
pub const HOME_MENU_FRIENDS_REQUEST_RECEIVED: LocalizedStringId = LocalizedStringId(178);
pub const HOME_MENU_FRIENDS_BLOCK_USER_MSG: LocalizedStringId = LocalizedStringId(179);
pub const HOME_MENU_FRIENDS_UNBLOCK_USER_MSG: LocalizedStringId = LocalizedStringId(180);
pub const HOME_MENU_FRIENDS_REMOVE_USER_MSG: LocalizedStringId = LocalizedStringId(181);
pub const HOME_MENU_FRIENDS_ACCEPT_REQUEST_MSG: LocalizedStringId = LocalizedStringId(182);
pub const HOME_MENU_FRIENDS_CANCEL_REQUEST_MSG: LocalizedStringId = LocalizedStringId(183);
pub const HOME_MENU_FRIENDS_REJECT_REQUEST_MSG: LocalizedStringId = LocalizedStringId(184);
pub const HOME_MENU_FRIENDS_REJECT_REQUEST: LocalizedStringId = LocalizedStringId(185);
pub const HOME_MENU_FRIENDS_NEXT_LIST: LocalizedStringId = LocalizedStringId(186);
pub const HOME_MENU_SETTINGS: LocalizedStringId = LocalizedStringId(187);
pub const HOME_MENU_SETTINGS_SAVE: LocalizedStringId = LocalizedStringId(188);
pub const HOME_MENU_SETTINGS_SAVE_BUTTON: LocalizedStringId = LocalizedStringId(189);
pub const HOME_MENU_SETTINGS_DISCARD: LocalizedStringId = LocalizedStringId(190);
pub const HOME_MENU_SETTINGS_DISCARD_BUTTON: LocalizedStringId = LocalizedStringId(191);
pub const HOME_MENU_SETTINGS_RESET_BUTTON: LocalizedStringId = LocalizedStringId(192);
pub const HOME_MENU_SETTINGS_AUDIO: LocalizedStringId = LocalizedStringId(193);
pub const HOME_MENU_SETTINGS_AUDIO_MASTER_VOLUME: LocalizedStringId = LocalizedStringId(194);
pub const HOME_MENU_SETTINGS_AUDIO_BACKEND: LocalizedStringId = LocalizedStringId(195);
pub const HOME_MENU_SETTINGS_AUDIO_BUFFERING: LocalizedStringId = LocalizedStringId(196);
pub const HOME_MENU_SETTINGS_AUDIO_BUFFER_DURATION: LocalizedStringId = LocalizedStringId(197);
pub const HOME_MENU_SETTINGS_AUDIO_TIME_STRETCHING: LocalizedStringId = LocalizedStringId(198);
pub const HOME_MENU_SETTINGS_AUDIO_TIME_STRETCHING_THRESHOLD: LocalizedStringId = LocalizedStringId(199);
pub const HOME_MENU_SETTINGS_VIDEO: LocalizedStringId = LocalizedStringId(200);
pub const HOME_MENU_SETTINGS_VIDEO_VSYNC: LocalizedStringId = LocalizedStringId(201);
pub const HOME_MENU_SETTINGS_VIDEO_FRAME_LIMIT: LocalizedStringId = LocalizedStringId(202);
pub const HOME_MENU_SETTINGS_VIDEO_ANISOTROPIC_OVERRIDE: LocalizedStringId = LocalizedStringId(203);
pub const HOME_MENU_SETTINGS_VIDEO_OUTPUT_SCALING: LocalizedStringId = LocalizedStringId(204);
pub const HOME_MENU_SETTINGS_VIDEO_RCAS_SHARPENING: LocalizedStringId = LocalizedStringId(205);
pub const HOME_MENU_SETTINGS_VIDEO_RESOLUTION_SCALE_PERCENT: LocalizedStringId = LocalizedStringId(206);
pub const HOME_MENU_SETTINGS_VIDEO_RESOLUTION_SCALE_THRESHOLD: LocalizedStringId = LocalizedStringId(207);
pub const HOME_MENU_SETTINGS_VIDEO_STRETCH_TO_DISPLAY: LocalizedStringId = LocalizedStringId(208);
pub const HOME_MENU_SETTINGS_VIDEO_STEREO_MODE: LocalizedStringId = LocalizedStringId(209);
pub const HOME_MENU_SETTINGS_INPUT: LocalizedStringId = LocalizedStringId(210);
pub const HOME_MENU_SETTINGS_INPUT_BACKGROUND_INPUT: LocalizedStringId = LocalizedStringId(211);
pub const HOME_MENU_SETTINGS_INPUT_KEEP_PADS_CONNECTED: LocalizedStringId = LocalizedStringId(212);
pub const HOME_MENU_SETTINGS_INPUT_SHOW_PS_MOVE_CURSOR: LocalizedStringId = LocalizedStringId(213);
pub const HOME_MENU_SETTINGS_INPUT_CAMERA_FLIP: LocalizedStringId = LocalizedStringId(214);
pub const HOME_MENU_SETTINGS_INPUT_PAD_MODE: LocalizedStringId = LocalizedStringId(215);
pub const HOME_MENU_SETTINGS_INPUT_PAD_SLEEP: LocalizedStringId = LocalizedStringId(216);
pub const HOME_MENU_SETTINGS_INPUT_FAKE_MOVE_ROTATION_CONE_H: LocalizedStringId = LocalizedStringId(217);
pub const HOME_MENU_SETTINGS_INPUT_FAKE_MOVE_ROTATION_CONE_V: LocalizedStringId = LocalizedStringId(218);
pub const HOME_MENU_SETTINGS_ADVANCED: LocalizedStringId = LocalizedStringId(219);
pub const HOME_MENU_SETTINGS_ADVANCED_PREFERRED_SPU_THREADS: LocalizedStringId = LocalizedStringId(220);
pub const HOME_MENU_SETTINGS_ADVANCED_MAX_CPU_PREEMPTIONS: LocalizedStringId = LocalizedStringId(221);
pub const HOME_MENU_SETTINGS_ADVANCED_ACCURATE_RSX_RESERVATION_ACCESS: LocalizedStringId = LocalizedStringId(222);
pub const HOME_MENU_SETTINGS_ADVANCED_SLEEP_TIMERS_ACCURACY: LocalizedStringId = LocalizedStringId(223);
pub const HOME_MENU_SETTINGS_ADVANCED_RSX_MEMORY_TILING: LocalizedStringId = LocalizedStringId(224);
pub const HOME_MENU_SETTINGS_ADVANCED_MAX_SPURS_THREADS: LocalizedStringId = LocalizedStringId(225);
pub const HOME_MENU_SETTINGS_ADVANCED_DRIVER_WAKE_UP_DELAY: LocalizedStringId = LocalizedStringId(226);
pub const HOME_MENU_SETTINGS_ADVANCED_VBLANK_FREQUENCY: LocalizedStringId = LocalizedStringId(227);
pub const HOME_MENU_SETTINGS_ADVANCED_VBLANK_NTSC: LocalizedStringId = LocalizedStringId(228);
pub const HOME_MENU_SETTINGS_OVERLAYS: LocalizedStringId = LocalizedStringId(229);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_TROPHY_POPUPS: LocalizedStringId = LocalizedStringId(230);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_RPCN_POPUPS: LocalizedStringId = LocalizedStringId(231);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_SHADER_COMPILATION_HINT: LocalizedStringId = LocalizedStringId(232);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_PPU_COMPILATION_HINT: LocalizedStringId = LocalizedStringId(233);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_AUTO_SAVE_LOAD_HINT: LocalizedStringId = LocalizedStringId(234);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_PRESSURE_INTENSITY_TOGGLE_HINT: LocalizedStringId = LocalizedStringId(235);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_ANALOG_LIMITER_TOGGLE_HINT: LocalizedStringId = LocalizedStringId(236);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_MOUSE_AND_KB_TOGGLE_HINT: LocalizedStringId = LocalizedStringId(237);
pub const HOME_MENU_SETTINGS_OVERLAYS_SHOW_FATAL_ERROR_HINTS: LocalizedStringId = LocalizedStringId(238);
pub const HOME_MENU_SETTINGS_OVERLAYS_RECORD_WITH_OVERLAYS: LocalizedStringId = LocalizedStringId(239);
pub const HOME_MENU_SETTINGS_OVERLAYS_PLAY_MUSIC_DURING_BOOT: LocalizedStringId = LocalizedStringId(240);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY: LocalizedStringId = LocalizedStringId(241);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_ENABLE: LocalizedStringId = LocalizedStringId(242);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_ENABLE_FRAMERATE_GRAPH: LocalizedStringId = LocalizedStringId(243);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_ENABLE_FRAMETIME_GRAPH: LocalizedStringId = LocalizedStringId(244);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_DETAIL_LEVEL: LocalizedStringId = LocalizedStringId(245);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_FRAMERATE_DETAIL_LEVEL: LocalizedStringId = LocalizedStringId(246);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_FRAMETIME_DETAIL_LEVEL: LocalizedStringId = LocalizedStringId(247);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_FRAMERATE_DATAPOINT_COUNT: LocalizedStringId = LocalizedStringId(248);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_FRAMETIME_DATAPOINT_COUNT: LocalizedStringId = LocalizedStringId(249);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_UPDATE_INTERVAL: LocalizedStringId = LocalizedStringId(250);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_POSITION: LocalizedStringId = LocalizedStringId(251);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_CENTER_X: LocalizedStringId = LocalizedStringId(252);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_CENTER_Y: LocalizedStringId = LocalizedStringId(253);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_MARGIN_X: LocalizedStringId = LocalizedStringId(254);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_MARGIN_Y: LocalizedStringId = LocalizedStringId(255);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_FONT_SIZE: LocalizedStringId = LocalizedStringId(256);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_OPACITY: LocalizedStringId = LocalizedStringId(257);
pub const HOME_MENU_SETTINGS_PERFORMANCE_OVERLAY_USE_WINDOW_SPACE: LocalizedStringId = LocalizedStringId(258);
pub const HOME_MENU_SETTINGS_DEBUG: LocalizedStringId = LocalizedStringId(259);
pub const HOME_MENU_SETTINGS_DEBUG_OVERLAY: LocalizedStringId = LocalizedStringId(260);
pub const HOME_MENU_SETTINGS_DEBUG_INPUT_OVERLAY: LocalizedStringId = LocalizedStringId(261);
pub const HOME_MENU_SETTINGS_MOUSE_DEBUG_INPUT_OVERLAY: LocalizedStringId = LocalizedStringId(262);
pub const HOME_MENU_SETTINGS_DEBUG_DISABLE_VIDEO_OUTPUT: LocalizedStringId = LocalizedStringId(263);
pub const HOME_MENU_SETTINGS_DEBUG_TEXTURE_LOD_BIAS: LocalizedStringId = LocalizedStringId(264);
pub const HOME_MENU_SCREENSHOT: LocalizedStringId = LocalizedStringId(265);
pub const HOME_MENU_SAVESTATE: LocalizedStringId = LocalizedStringId(266);
pub const HOME_MENU_SAVESTATE_SAVE: LocalizedStringId = LocalizedStringId(267);
pub const HOME_MENU_SAVESTATE_AND_EXIT: LocalizedStringId = LocalizedStringId(268);
pub const HOME_MENU_RELOAD_SAVESTATE: LocalizedStringId = LocalizedStringId(269);
pub const HOME_MENU_RELOAD_SECOND_SAVESTATE: LocalizedStringId = LocalizedStringId(270);
pub const HOME_MENU_RELOAD_THIRD_SAVESTATE: LocalizedStringId = LocalizedStringId(271);
pub const HOME_MENU_RELOAD_FOURTH_SAVESTATE: LocalizedStringId = LocalizedStringId(272);
pub const HOME_MENU_TOGGLE_FULLSCREEN: LocalizedStringId = LocalizedStringId(273);
pub const HOME_MENU_RECORDING: LocalizedStringId = LocalizedStringId(274);
pub const HOME_MENU_TROPHIES: LocalizedStringId = LocalizedStringId(275);
pub const HOME_MENU_TROPHY_LIST_TITLE: LocalizedStringId = LocalizedStringId(276);
pub const HOME_MENU_TROPHY_LOCKED_TITLE: LocalizedStringId = LocalizedStringId(277);
pub const HOME_MENU_TROPHY_HIDDEN_TITLE: LocalizedStringId = LocalizedStringId(278);
pub const HOME_MENU_TROPHY_HIDDEN_DESCRIPTION: LocalizedStringId = LocalizedStringId(279);
pub const HOME_MENU_TROPHY_SHOW_HIDDEN_TROPHIES: LocalizedStringId = LocalizedStringId(280);
pub const HOME_MENU_TROPHY_HIDE_HIDDEN_TROPHIES: LocalizedStringId = LocalizedStringId(281);
pub const HOME_MENU_TROPHY_PLATINUM_RELEVANT: LocalizedStringId = LocalizedStringId(282);
pub const HOME_MENU_TROPHY_GRADE_BRONZE: LocalizedStringId = LocalizedStringId(283);
pub const HOME_MENU_TROPHY_GRADE_SILVER: LocalizedStringId = LocalizedStringId(284);
pub const HOME_MENU_TROPHY_GRADE_GOLD: LocalizedStringId = LocalizedStringId(285);
pub const HOME_MENU_TROPHY_GRADE_PLATINUM: LocalizedStringId = LocalizedStringId(286);
pub const AUDIO_MUTED: LocalizedStringId = LocalizedStringId(287);
pub const AUDIO_UNMUTED: LocalizedStringId = LocalizedStringId(288);
pub const AUDIO_CHANGED: LocalizedStringId = LocalizedStringId(289);
pub const PROGRESS_DIALOG_PROGRESS: LocalizedStringId = LocalizedStringId(290);
pub const PROGRESS_DIALOG_PROGRESS_ANALYZING: LocalizedStringId = LocalizedStringId(291);
pub const PROGRESS_DIALOG_REMAINING: LocalizedStringId = LocalizedStringId(292);
pub const PROGRESS_DIALOG_DONE: LocalizedStringId = LocalizedStringId(293);
pub const PROGRESS_DIALOG_FILE: LocalizedStringId = LocalizedStringId(294);
pub const PROGRESS_DIALOG_MODULE: LocalizedStringId = LocalizedStringId(295);
pub const PROGRESS_DIALOG_OF: LocalizedStringId = LocalizedStringId(296);
pub const PROGRESS_DIALOG_PLEASE_WAIT: LocalizedStringId = LocalizedStringId(297);
pub const PROGRESS_DIALOG_STOPPING_PLEASE_WAIT: LocalizedStringId = LocalizedStringId(298);
pub const PROGRESS_DIALOG_SAVESTATE_PLEASE_WAIT: LocalizedStringId = LocalizedStringId(299);
pub const PROGRESS_DIALOG_SCANNING_PPU_EXECUTABLE: LocalizedStringId = LocalizedStringId(300);
pub const PROGRESS_DIALOG_ANALYZING_PPU_EXECUTABLE: LocalizedStringId = LocalizedStringId(301);
pub const PROGRESS_DIALOG_SCANNING_PPU_MODULES: LocalizedStringId = LocalizedStringId(302);
pub const PROGRESS_DIALOG_LOADING_PPU_MODULES: LocalizedStringId = LocalizedStringId(303);
pub const PROGRESS_DIALOG_COMPILING_PPU_MODULES: LocalizedStringId = LocalizedStringId(304);
pub const PROGRESS_DIALOG_LINKING_PPU_MODULES: LocalizedStringId = LocalizedStringId(305);
pub const PROGRESS_DIALOG_APPLYING_PPU_CODE: LocalizedStringId = LocalizedStringId(306);
pub const PROGRESS_DIALOG_BUILDING_SPU_CACHE: LocalizedStringId = LocalizedStringId(307);
pub const EMULATION_PAUSED_RESUME_WITH_START: LocalizedStringId = LocalizedStringId(308);
pub const EMULATION_RESUMING: LocalizedStringId = LocalizedStringId(309);
pub const EMULATION_FROZEN: LocalizedStringId = LocalizedStringId(310);
pub const SAVESTATE_FAILED_DUE_TO_VDEC: LocalizedStringId = LocalizedStringId(311);
pub const SAVESTATE_FAILED_DUE_TO_SAVEDATA: LocalizedStringId = LocalizedStringId(312);
pub const SAVESTATE_FAILED_DUE_TO_SPU: LocalizedStringId = LocalizedStringId(313);
pub const SAVESTATE_FAILED_DUE_TO_MISSING_SPU_SETTING: LocalizedStringId = LocalizedStringId(314);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_matches_cpp_header() {
        assert_eq!(LOCALIZED_STRING_ID_COUNT, 315);
    }

    #[test]
    fn invalid_is_zero() {
        assert_eq!(INVALID, LocalizedStringId(0));
    }

    #[test]
    fn last_id_is_savestate_spu_setting() {
        assert_eq!(
            SAVESTATE_FAILED_DUE_TO_MISSING_SPU_SETTING,
            LocalizedStringId(LOCALIZED_STRING_ID_COUNT - 1)
        );
        assert_eq!(SAVESTATE_FAILED_DUE_TO_MISSING_SPU_SETTING.0, 314);
    }

    #[test]
    fn rsx_overlays_block_starts_at_one() {
        // First non-INVALID variant (cpp header:7).
        assert_eq!(RSX_OVERLAYS_SPINNER_NO_TEXT.0, 1);
        assert_eq!(RSX_OVERLAYS_TROPHY_BRONZE.0, 2);
    }

    #[test]
    fn trophy_grade_block_contiguous() {
        // cpp enum block cpp:8..11 — Bronze through Platinum.
        assert_eq!(RSX_OVERLAYS_TROPHY_BRONZE.0, 2);
        assert_eq!(RSX_OVERLAYS_TROPHY_SILVER.0, 3);
        assert_eq!(RSX_OVERLAYS_TROPHY_GOLD.0, 4);
        assert_eq!(RSX_OVERLAYS_TROPHY_PLATINUM.0, 5);
    }

    #[test]
    fn mouse_and_keyboard_overlay_pair() {
        // Used by rpcs3-io-interception crate (cpp:65..67).
        // The cpp enum place them adjacent, emulated before pad.
        assert_eq!(
            RSX_OVERLAYS_MOUSE_AND_KEYBOARD_EMULATED.0 + 1,
            RSX_OVERLAYS_MOUSE_AND_KEYBOARD_PAD.0
        );
    }

    #[test]
    fn audio_group_adjacent_and_near_290() {
        assert_eq!(AUDIO_MUTED.0 + 1, AUDIO_UNMUTED.0);
        assert_eq!(AUDIO_UNMUTED.0 + 1, AUDIO_CHANGED.0);
    }

    #[test]
    fn repr_transparent_matches_u32_size() {
        use core::mem::size_of;
        assert_eq!(size_of::<LocalizedStringId>(), size_of::<u32>());
    }

    #[test]
    fn id_equality_via_inner_value() {
        let a = LocalizedStringId(42);
        let b = LocalizedStringId(42);
        let c = LocalizedStringId(43);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn discriminants_are_dense_no_gaps() {
        // Spot-check a few strategic positions to ensure no
        // accidental gap: positions 0, 34, 114, 265, 314.
        assert_eq!(INVALID.0, 0);
        assert_eq!(RSX_OVERLAYS_MOUSE_AND_KEYBOARD_PAD.0, 34);
        assert_eq!(CELL_OSK_DIALOG_TITLE.0, 114);
        assert_eq!(HOME_MENU_SCREENSHOT.0, 265);
        assert_eq!(SAVESTATE_FAILED_DUE_TO_MISSING_SPU_SETTING.0, 314);
    }
}
