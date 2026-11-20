//! The SSH area: servers in nested groups with inherited settings, port forwards, keys and known
//! servers. Sessions open in KalangoOS's Terminal (`kalango-shell terminal-ssh <host-id>`), which reads
//! `hosts.json` and asks this app for the command (`kalango-remote ssh-command <host-id>`).

pub mod model;
pub mod openssh;
pub mod ops;
pub mod page;
