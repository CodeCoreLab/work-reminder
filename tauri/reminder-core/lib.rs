// Copyright (c) 2026 Ma Xin <maxin.zhang@users.noreply.github.com>
// SPDX-License-Identifier: MIT
//
// Platform-independent core: time arithmetic, task model, scheduling, tick loop
// and persistence. Contains no Win32 API calls, so it is unit tested on any host
// and shared verbatim by every front end.

pub mod engine;
pub mod legacy;
pub mod model;
pub mod scheduler;
pub mod store;
pub mod time;
