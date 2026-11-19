//! Public-API tests for synthetic mouse input, driven through
//! [`RecordingMouse`] so they run on any OS with no window, pointer, or
//! permission of any kind.

use framewatch::input::{
    execute, plan, CoordMap, CoordSpace, InputRequest, MouseAction, MouseBackend, MouseButton,
    MouseOp, PlanSettings, Point, RecordingMouse,
};
use framewatch::{InputError, Rect};

/// Timing off: assertions are about the operations, not the pacing.
fn instant() -> PlanSettings {
    PlanSettings {
        press_ms: 0,
        multi_click_gap_ms: 0,
        between_ms: 0,
        move_settle_ms: 0,
        drag_step_ms: 0,
        drag_steps: 4,
    }
}

/// A 800x600 window whose top-left sits at screen (100, 50).
fn window() -> CoordMap {
    CoordMap::window(Rect::new(100, 50, 800, 600))
}

fn run(actions: &[MouseAction], map: &CoordMap) -> Vec<MouseOp> {
    let ops = plan(actions, map, &instant()).expect("plan");
    let mut mouse = RecordingMouse::default();
    execute(&ops, &mut mouse).expect("execute");
    // What the backend saw must be exactly what was planned, minus the sleeps.
    let performed: Vec<MouseOp> = ops
        .iter()
        .copied()
        .filter(|op| !matches!(op, MouseOp::Sleep { .. }))
        .collect();
    assert_eq!(mouse.ops, performed);
    ops
}

#[test]
fn a_window_space_click_lands_at_the_window_offset() {
    let ops = run(
        &[MouseAction::Click {
            button: MouseButton::Left,
            at: Some(Point::new(40, 30)),
            count: 1,
        }],
        &window(),
    );
    assert_eq!(
        ops.first(),
        Some(&MouseOp::MoveTo {
            to: Point::new(140, 80)
        })
    );
}

#[test]
fn a_cropped_capture_keeps_coordinates_read_off_the_crop() {
    // Captured with `--roi 200,100,400,300`; the agent measured (10, 10) in
    // that cropped image, which is screen (100+200+10, 50+100+10).
    let map = window().with_roi(Rect::new(200, 100, 400, 300));
    let ops = run(
        &[MouseAction::Move {
            to: Point::new(10, 10),
        }],
        &map,
    );
    assert_eq!(
        ops,
        vec![MouseOp::MoveTo {
            to: Point::new(310, 160)
        }]
    );
}

#[test]
fn a_point_off_the_window_is_refused_before_anything_moves() {
    let mut mouse = RecordingMouse::default();
    let error = plan(
        &[
            MouseAction::Move {
                to: Point::new(1, 1),
            },
            MouseAction::Click {
                button: MouseButton::Left,
                at: Some(Point::new(900, 1)),
                count: 1,
            },
        ],
        &window(),
        &instant(),
    )
    .unwrap_err();
    assert!(matches!(error, InputError::OutOfBounds { .. }));
    // The rejected plan never reached a backend, so the valid first action did
    // not happen either: a bad request is all-or-nothing.
    assert!(mouse.ops.is_empty());
    assert_eq!(mouse.cursor_position().unwrap(), Point::new(0, 0));
}

#[test]
fn a_drag_moves_through_the_middle_rather_than_teleporting() {
    let ops = run(
        &[MouseAction::Drag {
            from: Some(Point::new(0, 0)),
            to: Point::new(100, 100),
            button: MouseButton::Left,
            steps: 4,
            hold_ms: 0,
        }],
        &window(),
    );
    let moves: Vec<Point> = ops
        .iter()
        .filter_map(|op| match op {
            MouseOp::MoveTo { to } => Some(*to),
            _ => None,
        })
        .collect();
    assert_eq!(moves.len(), 5, "the start plus four interpolated steps");
    assert_eq!(moves.last(), Some(&Point::new(200, 150)));
    // The button is held across every intermediate move.
    let down = ops
        .iter()
        .position(|op| matches!(op, MouseOp::Button { down: true, .. }))
        .unwrap();
    let up = ops
        .iter()
        .position(|op| matches!(op, MouseOp::Button { down: false, .. }))
        .unwrap();
    assert!(down < up);
    assert_eq!(
        ops[down + 1..up]
            .iter()
            .filter(|op| matches!(op, MouseOp::MoveTo { .. }))
            .count(),
        4
    );
}

#[test]
fn screen_space_needs_no_window_and_no_bounds() {
    let ops = run(
        &[MouseAction::Move {
            to: Point::new(3000, -20),
        }],
        &CoordMap::screen(),
    );
    assert_eq!(
        ops,
        vec![MouseOp::MoveTo {
            to: Point::new(3000, -20)
        }]
    );
}

#[test]
fn an_action_script_deserializes_from_the_documented_json() {
    let script = r#"[
        {"action": "move", "to": [10, 10]},
        {"action": "press"},
        {"action": "move", "to": [60, 10]},
        {"action": "release"}
    ]"#;
    let actions: Vec<MouseAction> = serde_json::from_str(script).unwrap();
    let ops = run(&actions, &window());
    assert_eq!(
        ops,
        vec![
            MouseOp::MoveTo {
                to: Point::new(110, 60)
            },
            MouseOp::Button {
                button: MouseButton::Left,
                down: true,
                click_index: 1
            },
            MouseOp::MoveTo {
                to: Point::new(160, 60)
            },
            MouseOp::Button {
                button: MouseButton::Left,
                down: false,
                click_index: 1
            },
        ]
    );
}

#[test]
fn a_request_in_window_space_without_a_target_is_refused() {
    let request = InputRequest::new(vec![MouseAction::Move {
        to: Point::new(1, 1),
    }]);
    let error = framewatch::input::dry_run(&request).unwrap_err();
    assert!(matches!(
        error,
        framewatch::Error::Input(InputError::Request(_))
    ));
}

#[test]
fn a_dry_run_in_screen_space_plans_without_a_backend() {
    let request = InputRequest::new(vec![
        MouseAction::Move {
            to: Point::new(10, 10),
        },
        MouseAction::Click {
            button: MouseButton::Middle,
            at: None,
            count: 3,
        },
    ])
    .space(CoordSpace::Screen);
    let outcome = framewatch::input::dry_run(&request).unwrap();
    assert!(!outcome.performed);
    assert_eq!(outcome.cursor_screen, Point::new(10, 10));
    assert_eq!(
        outcome
            .ops
            .iter()
            .filter(|op| matches!(op, MouseOp::Button { down: true, .. }))
            .count(),
        3
    );
}
