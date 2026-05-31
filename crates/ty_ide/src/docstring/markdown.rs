/// Returns the opening fence marker for a Markdown fenced code block.
pub(super) fn fence_start(line: &str) -> Option<&str> {
    let line = line.trim_start_matches(' ');
    let has_tick_fence = line.starts_with("```");
    let has_tilde_fence = line.starts_with("~~~");
    if !has_tick_fence && !has_tilde_fence {
        return None;
    }

    let without_leading_fence = if has_tick_fence {
        line.trim_start_matches('`')
    } else {
        line.trim_start_matches('~')
    };
    let fence_len = line.len() - without_leading_fence.len();
    let fence = &line[..fence_len];

    // An opener's info string cannot contain the same marker that closes it.
    (!without_leading_fence.contains(fence)).then_some(fence)
}

/// Returns whether `line` closes a fenced code block opened by `fence`.
pub(super) fn closes_fence(line: &str, fence: &str) -> bool {
    line.trim_start_matches(' ').starts_with(fence)
}
