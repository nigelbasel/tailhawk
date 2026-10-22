//{{NO_DEPENDENCIES}}
// Control ids for the Remote sources wizard mock-up.
//
// **This file is a drawing, not the product.** Tailhawk builds its dialogs from `Item` lists in
// `crates/tailhawk/src/dialog.rs` — hand-written templates, because a `.res` would mean a resource
// compiler in the build and `SPEC.md` §3.2 keeps the binary free of one. These resources exist so
// the layout can be opened in the Visual Studio dialog designer, moved around by eye, and handed
// back; the numbers here are then transcribed into `wizard_dialog_items()`.
//
// The ids deliberately match the ones the shell will use, so a control moved in the designer can be
// matched to its handler without translation.

#define IDC_STATIC                      -1

// **The window styles, spelled out, so the `.rc` includes nothing.** Opened as a loose file rather
// than as part of a project, the resource editor has no SDK include path and `#include <windows.h>`
// fails with **RC1015: cannot open include file 'windows.h'**. These are the handful of constants
// the five pages actually use, with their documented values; the guards mean the file still behaves
// if it is ever pulled into something that does include the real headers.
#ifndef WS_POPUP
#define WS_POPUP                        0x80000000L
#define WS_CAPTION                      0x00C00000L
#define WS_SYSMENU                      0x00080000L
#define WS_VISIBLE                      0x10000000L
#define WS_BORDER                       0x00800000L
#define WS_VSCROLL                      0x00200000L
#define WS_DISABLED                     0x08000000L
#define WS_GROUP                        0x00020000L
#define WS_TABSTOP                      0x00010000L
#endif

#ifndef DS_SETFONT
#define DS_SETFONT                      0x0040L
#define DS_MODALFRAME                   0x0080L
#endif

#ifndef ES_AUTOHSCROLL
#define ES_AUTOHSCROLL                  0x0080L
#define ES_PASSWORD                     0x0020L
#endif

#ifndef BS_AUTORADIOBUTTON
#define BS_AUTORADIOBUTTON              0x0009L
#endif

#ifndef LBS_NOINTEGRALHEIGHT
#define LBS_NOINTEGRALHEIGHT            0x0100L
#endif

#ifndef CBS_DROPDOWNLIST
#define CBS_DROPDOWNLIST                0x0003L
#endif

// A tree view's styles. `SysTreeView32` has no ordinal either, so the page names the class as a
// string and the designer draws a generic control.
#ifndef TVS_HASBUTTONS
#define TVS_HASBUTTONS                  0x0001L
#define TVS_HASLINES                    0x0002L
#define TVS_LINESATROOT                 0x0004L
#define TVS_SHOWSELALWAYS               0x0020L
#endif

// A list view's styles. `SysListView32` has no ordinal, so the pages name the class as a string and
// the designer draws it as a generic control — which is as much as a drawing needs. The check boxes
// on the values list are `LVS_EX_CHECKBOXES`, an *extended* style the shell sets at run time, so
// there is nothing here to draw them with.
#ifndef LVS_REPORT
#define LVS_REPORT                      0x0001L
#define LVS_SHOWSELALWAYS               0x0008L
#define LVS_NOCOLUMNHEADER              0x4000L
#endif

#ifndef IDCANCEL
#define IDCANCEL                        2
#endif

// The Remote sources dialog — a tree of sources with their tails beneath them.
#define IDC_SRC_TREE                    460
#define IDC_SRC_NEW_SOURCE              461
#define IDC_SRC_ADD_TAIL                462
#define IDC_SRC_EDIT                    463
#define IDC_SRC_CLONE                   464
#define IDC_SRC_DELETE                  465
#define IDC_SRC_FAULT                   466
#define IDC_SRC_CLOSE                   467

// The frame, shared by every page.
#define IDC_WIZ_HEADING                 400
#define IDC_WIZ_STEP                    401
#define IDC_WIZ_SAYS                    402
#define IDC_WIZ_BACK                    403
#define IDC_WIZ_NEXT                    404

// Page 1 — Which log server?
#define IDC_WIZ_NAME                    410
#define IDC_WIZ_URL                     411
#define IDC_WIZ_KNOWN                   412
#define IDC_WIZ_SERVERS                 413
#define IDC_WIZ_OTHER                   414

// Page 2 — How should Tailhawk sign in?
#define IDC_WIZ_FOUND                   420
#define IDC_WIZ_BROWSER                 421
#define IDC_WIZ_SECRET_MODE             422
#define IDC_WIZ_OPEN                    423
#define IDC_WIZ_TOKEN                   424
#define IDC_WIZ_AUTH                    425
#define IDC_WIZ_SCOPE                   426

// Page 3 — What is this client called?
#define IDC_WIZ_CLIENT                  430
#define IDC_WIZ_SECRET                  431

// Page 4 — What should this source show?
#define IDC_WIZ_LABELS                  440
#define IDC_WIZ_VALUES                  441
#define IDC_WIZ_SHOWS                   442

// Page 5 — Ready to use
#define IDC_WIZ_SUMMARY                 450
#define IDC_WIZ_TRY                     451
#define IDC_WIZ_TRIED                   452
