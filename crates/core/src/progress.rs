//! Watch progress: one SQLite file per drive, kept ON the drive (`.courseshelf/progress.sqlite`)
//! so it travels with the disk.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

pub const DONE_FRACTION: f64 = 0.95;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Prog {
    pub position: f64,
    pub duration: f64,
    pub done: bool,
    pub last_watched: i64,
}

pub struct Progress {
    conn: Connection,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Progress {
    pub fn open(state_dir: &Path) -> Result<Progress> {
        std::fs::create_dir_all(state_dir)?;
        let conn = Connection::open(state_dir.join("progress.sqlite"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS lecture (
                course TEXT NOT NULL, key TEXT NOT NULL,
                position REAL NOT NULL DEFAULT 0, duration REAL NOT NULL DEFAULT 0,
                done INTEGER NOT NULL DEFAULT 0, last_watched INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (course, key));
             CREATE TABLE IF NOT EXISTS course_pref (
                course TEXT PRIMARY KEY, speed REAL NOT NULL DEFAULT 1.0);",
        )?;
        Ok(Progress { conn })
    }

    pub fn all(&self, course: &str) -> Result<HashMap<String, Prog>> {
        let mut st = self.conn.prepare("SELECT key, position, duration, done, last_watched FROM lecture WHERE course = ?1")?;
        let rows = st.query_map([course], |r| {
            Ok((r.get::<_, String>(0)?, Prog { position: r.get(1)?, duration: r.get(2)?, done: r.get::<_, i64>(3)? != 0, last_watched: r.get(4)? }))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Record a position. Marks the lecture done once 95% is reached; never un-marks it.
    pub fn save(&self, course: &str, key: &str, position: f64, duration: f64) -> Result<bool> {
        let position = if position.is_finite() && position > 0.0 { position } else { 0.0 };
        let duration = if duration.is_finite() && duration > 0.0 { duration } else { 0.0 };
        let reached = duration > 0.0 && position / duration >= DONE_FRACTION;
        self.conn.execute(
            "INSERT INTO lecture (course, key, position, duration, done, last_watched) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(course, key) DO UPDATE SET position = ?3,
                duration = CASE WHEN ?4 > 0 THEN ?4 ELSE duration END,
                done = MAX(done, ?5), last_watched = ?6",
            params![course, key, position, duration, reached as i64, now()],
        )?;
        let done: i64 = self.conn.query_row("SELECT done FROM lecture WHERE course = ?1 AND key = ?2", params![course, key], |r| r.get(0))?;
        Ok(done != 0)
    }

    pub fn set_done(&self, course: &str, key: &str, done: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO lecture (course, key, done, last_watched) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(course, key) DO UPDATE SET done = ?3, last_watched = ?4",
            params![course, key, done as i64, now()],
        )?;
        Ok(())
    }

    /// Every lecture row of a course (for moving progress to another drive).
    pub fn export(&self, course: &str) -> Result<Vec<(String, Prog)>> {
        Ok(self.all(course)?.into_iter().collect())
    }

    /// Insert rows for a course, keeping the newer one if the lecture is already there.
    pub fn import(&self, course: &str, rows: &[(String, Prog)], speed: Option<f64>) -> Result<usize> {
        let mut n = 0;
        for (key, p) in rows {
            n += self.conn.execute(
                "INSERT INTO lecture (course, key, position, duration, done, last_watched) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(course, key) DO UPDATE SET position = ?3, duration = ?4, done = ?5, last_watched = ?6
                 WHERE excluded.last_watched >= lecture.last_watched",
                params![course, key, p.position, p.duration, p.done as i64, p.last_watched],
            )?;
        }
        if let Some(sp) = speed {
            self.set_speed(course, sp)?;
        }
        Ok(n)
    }

    /// A file was renamed: its progress (keyed by the old stem when there is no .info.json) follows it.
    pub fn rename_key(&self, course: &str, old: &str, new: &str) -> Result<()> {
        self.conn.execute("UPDATE OR IGNORE lecture SET key = ?3 WHERE course = ?1 AND key = ?2", params![course, old, new])?;
        Ok(())
    }

    pub fn speed(&self, course: &str) -> f64 {
        self.conn.query_row("SELECT speed FROM course_pref WHERE course = ?1", [course], |r| r.get(0)).unwrap_or(1.0)
    }

    pub fn set_speed(&self, course: &str, speed: f64) -> Result<()> {
        let speed = speed.clamp(0.25, 4.0);
        self.conn.execute(
            "INSERT INTO course_pref (course, speed) VALUES (?1, ?2) ON CONFLICT(course) DO UPDATE SET speed = ?2",
            params![course, speed],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cs_prog_{}_{}", std::process::id(), now()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn saves_resumes_and_marks_done() {
        let d = tmp();
        let p = Progress::open(&d).unwrap();
        assert!(!p.save("C", "k1", 100.0, 1000.0).unwrap());
        assert_eq!(p.all("C").unwrap()["k1"].position, 100.0);
        assert!(p.save("C", "k1", 960.0, 1000.0).unwrap());
        // going back never un-marks
        assert!(p.save("C", "k1", 10.0, 1000.0).unwrap());
        p.set_done("C", "k1", false).unwrap();
        assert!(!p.all("C").unwrap()["k1"].done);
        assert_eq!(p.speed("C"), 1.0);
        p.set_speed("C", 1.5).unwrap();
        assert_eq!(p.speed("C"), 1.5);
        // export -> import into another database, and rename_key
        let d2 = tmp().with_extension("b");
        let q = Progress::open(&d2).unwrap();
        let rows = p.export("C").unwrap();
        assert_eq!(q.import("C", &rows, Some(1.5)).unwrap(), 1);
        assert_eq!(q.all("C").unwrap()["k1"].position, 10.0);
        q.rename_key("C", "k1", "k2").unwrap();
        assert!(q.all("C").unwrap().contains_key("k2") && !q.all("C").unwrap().contains_key("k1"));
        let _ = std::fs::remove_dir_all(&d2);
        let _ = std::fs::remove_dir_all(&d);
    }
}
