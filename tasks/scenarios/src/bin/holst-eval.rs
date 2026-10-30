//! `holst-eval`: oracle, serve and agent modes of the MCP agent eval.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command line tool: it reads its arguments and reports on the terminal"
)]

use std::{path::PathBuf, process::ExitCode};

use holst_scenarios::{
    ScenarioError,
    eval::{
        AgentRun,
        Scored,
        TASKS,
        fixture,
        oracle,
        run_agent,
        write_config,
        write_scores,
    },
};

#[derive(Debug, thiserror::Error)]
enum EvalError {
    #[error(
        "usage: holst-eval oracle | serve | agent --command '<agent command with {{prompt}} and {{config}}>' [--runs 3] --out <dir>"
    )]
    Usage,
    #[error("{0}")]
    Scenario(#[from] ScenarioError),
    #[error("--runs: {0}")]
    Runs(std::num::ParseIntError),
    #[error("{0} tasks not proven")]
    Unproven(usize),
    #[error("{0}")]
    Signal(std::io::Error),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.split_first() {
        Some((mode, rest)) if mode == "oracle" && rest.is_empty() => prove().await,
        Some((mode, rest)) if mode == "serve" && rest.is_empty() => serve().await,
        Some((mode, rest)) if mode == "agent" => agent(rest).await,
        _ => Err(EvalError::Usage),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn prove() -> Result<(), EvalError> {
    let lines = oracle().await.map_err(EvalError::Scenario)?;
    // imperative: prints line by line
    for line in &lines {
        let right = line
            .right_answer
            .as_ref()
            .map_or_else(|why| format!("FAILS: {why}"), |()| "passes".to_owned());
        let untouched = line
            .untouched_fixture
            .as_ref()
            .map_or("fails", |()| "PASSES");
        println!(
            "task {:2}: calls {} (ideal {}); right answer {right}; untouched fixture {untouched}",
            line.number, line.calls, line.ideal
        );
    }
    println!("skipped: none");
    let broken = lines.iter().filter(|line| !line.proven()).count();
    (broken == 0)
        .then_some(())
        .ok_or(EvalError::Unproven(broken))
}

async fn serve() -> Result<(), EvalError> {
    let holst = fixture().await.map_err(EvalError::Scenario)?;
    let config = write_config(&holst).map_err(EvalError::Scenario)?;
    println!("MCP config: {}", config.display());
    println!("Ctrl-C to stop");
    tokio::signal::ctrl_c().await.map_err(EvalError::Signal)
}

fn option<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments.windows(2).find_map(|pair| match pair {
        [flag, value] if flag == name => Some(value.as_str()),
        _ => None,
    })
}

async fn agent(arguments: &[String]) -> Result<(), EvalError> {
    let run = AgentRun {
        command: option(arguments, "--command")
            .ok_or(EvalError::Usage)?
            .to_owned(),
        runs: option(arguments, "--runs")
            .map_or(Ok(3), str::parse)
            .map_err(EvalError::Runs)?,
        out: PathBuf::from(option(arguments, "--out").ok_or(EvalError::Usage)?),
    };
    let mut scored: Vec<Scored> = Vec::new();
    // imperative: one agent run after the other, each on its own fixture
    for index in 0..TASKS {
        // imperative: runs of one task, in order
        for number in 1..=run.runs {
            let result = run_agent(index, &run, number)
                .await
                .map_err(EvalError::Scenario)?;
            let reported: Vec<String> = result
                .iter()
                .map(|done| {
                    format!(
                        "task {:2} run {number}: {} calls, correct: {}",
                        done.number,
                        done.uses.len(),
                        done.verdict.correct
                    )
                })
                .collect();
            println!("{}", reported.join("\n"));
            scored.extend(result);
        }
    }
    write_scores(&run.out, &scored).map_err(EvalError::Scenario)
}
