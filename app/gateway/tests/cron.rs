//! Cron scheduler tests.

use walrus_gateway::config::CronConfig;
use walrus_gateway::{CronJob, CronScheduler};

#[test]
fn parse_valid_cron_expression() {
    let config = CronConfig {
        name: "test".into(),
        schedule: "0 0 9 * * *".to_string(),
        agent: "assistant".into(),
        message: "hello".to_string(),
    };
    let job = CronJob::from_config(&config).unwrap();
    assert_eq!(job.name.as_str(), "test");
    assert_eq!(job.agent.as_str(), "assistant");
    assert_eq!(job.message, "hello");
}

#[test]
fn invalid_cron_expression() {
    let config = CronConfig {
        name: "bad".into(),
        schedule: "not a cron".to_string(),
        agent: "assistant".into(),
        message: "hello".to_string(),
    };
    assert!(CronJob::from_config(&config).is_err());
}

#[test]
fn scheduler_from_configs() {
    let configs = vec![
        CronConfig {
            name: "job1".into(),
            schedule: "0 0 * * * *".to_string(),
            agent: "a".into(),
            message: "m1".to_string(),
        },
        CronConfig {
            name: "job2".into(),
            schedule: "0 30 * * * *".to_string(),
            agent: "b".into(),
            message: "m2".to_string(),
        },
    ];
    let scheduler = CronScheduler::from_configs(&configs).unwrap();
    assert_eq!(scheduler.jobs().len(), 2);
}

#[test]
fn scheduler_empty_jobs() {
    let scheduler = CronScheduler::new(vec![]);
    assert!(scheduler.jobs().is_empty());
}

#[test]
fn scheduler_invalid_config_fails() {
    let configs = vec![CronConfig {
        name: "bad".into(),
        schedule: "invalid".to_string(),
        agent: "a".into(),
        message: "m".to_string(),
    }];
    assert!(CronScheduler::from_configs(&configs).is_err());
}
