"""Tab panes for displaying workflow run details, jobs, steps, and logs."""

from datetime import datetime

from textual import work
from textual.app import ComposeResult
from textual.containers import ScrollableContainer, VerticalScroll
from textual.content import Content
from textual.widgets import Button, Label, Markdown, Rule, TabPane, Tree

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.workflows import (
    get_job_logs,
    get_workflow_jobs,
    rerun_failed_jobs,
    rerun_workflow,
)
from lazy_github.lib.messages import WorkflowRunSelected
from lazy_github.models.github import (
    WorkflowJob,
    WorkflowJobConclusion,
    WorkflowJobStatus,
    WorkflowRun,
    WorkflowStep,
    WorkflowStepConclusion,
    WorkflowStepStatus,
)


class WorkflowRunOverviewTabPane(TabPane):
    """Overview tab showing workflow run metadata, commit info, and job summaries."""

    DEFAULT_CSS = """
    WorkflowRunOverviewTabPane {
        overflow-y: auto;
    }

    WorkflowRunOverviewTabPane Button {
        margin: 1 2;
        width: auto;
    }

    Collapsible {
        height: auto;
    }
    """

    def __init__(self, workflow_run: WorkflowRun) -> None:
        super().__init__("Overview", id="workflow_run_overview_pane")
        self.workflow_run = workflow_run
        self.jobs: list[WorkflowJob] = []

    def on_mount(self) -> None:
        """Load jobs when the pane is mounted."""
        self.load_jobs()

    @work
    async def load_jobs(self) -> None:
        """Load jobs for this workflow run."""
        self.jobs = await get_workflow_jobs(self.workflow_run.repository, self.workflow_run.id)
        await self.refresh_job_summary()

    async def refresh_job_summary(self) -> None:
        """Refresh the job summary section."""
        job_summary_container = self.query_one("#job_summary_container")
        await job_summary_container.remove_children()

        if not self.jobs:
            await job_summary_container.mount(Label("No jobs found"))
            return

        # Check if there are any failed jobs
        has_failed_jobs = any(
            job.status == WorkflowJobStatus.COMPLETED and job.conclusion == WorkflowJobConclusion.FAILURE
            for job in self.jobs
        )

        # Show/hide the re-run failed jobs button
        try:
            rerun_failed_button = self.query_one("#rerun_failed_jobs", Button)
            rerun_failed_button.display = has_failed_jobs
        except Exception:
            # Button might not exist yet if jobs load before compose completes
            pass

        for job in self.jobs:
            status_display = self._format_job_status(job)
            duration = self._calculate_duration(job.started_at, job.completed_at)
            job_info = f"{status_display} {job.name} ({duration})"
            await job_summary_container.mount(Label(Content.from_markup(job_info)))

    def _format_job_status(self, job: WorkflowJob) -> str:
        """Format job status with colors and symbols."""
        if job.status == WorkflowJobStatus.COMPLETED:
            if job.conclusion == WorkflowJobConclusion.SUCCESS:
                return "[greenyellow]✓[/]"
            elif job.conclusion == WorkflowJobConclusion.FAILURE:
                return "[red]✗[/]"
            elif job.conclusion == WorkflowJobConclusion.CANCELLED:
                return "[yellow]○[/]"
            elif job.conclusion == WorkflowJobConclusion.SKIPPED:
                return "[dim]−[/]"
        elif job.status == WorkflowJobStatus.IN_PROGRESS:
            return "[cyan]⋯[/]"
        elif job.status == WorkflowJobStatus.QUEUED:
            return "[yellow]⋯[/]"

        return "[dim]?[/]"

    def _format_run_status(self, run: WorkflowRun) -> str:
        """Format overall run status with colors."""
        if run.status == "completed":
            if run.conclusion == "success":
                return "[greenyellow]✓ Success[/]"
            elif run.conclusion == "failure":
                return "[red]✗ Failure[/]"
            elif run.conclusion == "cancelled":
                return "[yellow]○ Cancelled[/]"
            elif run.conclusion == "skipped":
                return "[dim]− Skipped[/]"
        elif run.status == "in_progress":
            return "[cyan]⋯ In Progress[/]"
        elif run.status == "queued":
            return "[yellow]⋯ Queued[/]"

        return f"[dim]{run.status}[/]"

    def _calculate_duration(self, started_at: datetime | None, completed_at: datetime | None) -> str:
        """Calculate and format duration between two datetime objects."""
        if not started_at:
            return "Not started"
        if not completed_at:
            return "In progress"

        duration = completed_at - started_at
        total_seconds = int(duration.total_seconds())

        if total_seconds < 60:
            return f"{total_seconds}s"
        elif total_seconds < 3600:
            minutes = total_seconds // 60
            seconds = total_seconds % 60
            return f"{minutes}m {seconds}s"
        else:
            hours = total_seconds // 3600
            minutes = (total_seconds % 3600) // 60
            return f"{hours}h {minutes}m"

    @work
    async def on_button_pressed(self, event: Button.Pressed) -> None:
        """Handle button presses for re-run actions."""
        if event.button.id == "rerun_workflow":
            success = await rerun_workflow(self.workflow_run.repository, self.workflow_run.id)
            if success:
                self.notify("Workflow run queued for re-run", title="Re-run Started")
                # Refresh the workflow run details
                self.post_message(WorkflowRunSelected(self.workflow_run))
            else:
                self.notify("Failed to re-run workflow", title="Error", severity="error")

        elif event.button.id == "rerun_failed_jobs":
            success = await rerun_failed_jobs(self.workflow_run.repository, self.workflow_run.id)
            if success:
                self.notify("Failed jobs queued for re-run", title="Re-run Started")
                self.post_message(WorkflowRunSelected(self.workflow_run))
            else:
                self.notify("Failed to re-run failed jobs", title="Error", severity="error")

    def compose(self) -> ComposeResult:
        run = self.workflow_run
        run_link = f'[link="{run.html_url}"](#{run.run_number})[/link]'

        with ScrollableContainer():
            # Status and title
            status_display = self._format_run_status(run)
            yield Label(Content.from_markup(f"{status_display} [b]$title[/] {run_link}", title=run.display_title))

            # Run metadata
            created_date = run.created_at.strftime("%Y-%m-%d %H:%M:%S")
            updated_date = run.updated_at.strftime("%Y-%m-%d %H:%M:%S")

            metadata_lines = [
                f"**Run Number:** {run.run_number}",
                f"**Event:** {run.event}",
                f"**Branch:** {run.head_branch}",
                f"**Status:** {run.status}",
            ]

            if run.conclusion:
                metadata_lines.append(f"**Conclusion:** {run.conclusion}")

            if run.run_attempt:
                metadata_lines.append(f"**Attempt:** {run.run_attempt}")

            metadata_lines.extend(
                [
                    f"**Created:** {created_date}",
                    f"**Updated:** {updated_date}",
                ]
            )

            yield Markdown("\n".join(metadata_lines))

            yield Rule()

            # Commit information
            yield Label("[b]Commit Information[/]")
            commit_sha_short = run.head_sha[:7]
            commit_lines = [
                f"**SHA:** `{commit_sha_short}` ({run.head_sha})",
                f"**Triggered by:** {run.triggering_actor.login}",
                f"**Actor:** {run.actor.login}",
            ]
            yield Markdown("\n".join(commit_lines))

            yield Rule()

            # Job summary
            yield Label("[b]Jobs Summary[/]")
            with VerticalScroll(id="job_summary_container"):
                yield Label("Loading jobs...")

            yield Rule()

            # Re-run buttons
            yield Label("[b]Actions[/]")
            with VerticalScroll(id="rerun_buttons"):
                yield Button("Re-run Workflow", id="rerun_workflow", variant="primary")
                # This button will be hidden by default and shown only if there are failed jobs
                rerun_failed_button = Button("Re-run Failed Jobs", id="rerun_failed_jobs", variant="default")
                rerun_failed_button.display = False
                yield rerun_failed_button


class WorkflowRunJobsTabPane(TabPane):
    """Tab showing jobs and their steps with the ability to view individual logs."""

    BINDINGS = [LazyGithubBindings.VIEW_JOB_LOGS]

    DEFAULT_CSS = """
    WorkflowRunJobsTabPane {
        overflow-y: auto;
    }

    WorkflowRunJobsTabPane Tree {
        height: 1fr;
        min-height: 20;
    }
    """

    def __init__(self, workflow_run: WorkflowRun) -> None:
        super().__init__("Jobs", id="workflow_run_jobs_pane")
        self.workflow_run = workflow_run
        self.jobs: list[WorkflowJob] = []
        self.current_job_logs: str | None = None

    def on_mount(self) -> None:
        """Load jobs when the pane is mounted."""
        self.load_jobs()

    @work
    async def load_jobs(self) -> None:
        """Load jobs for this workflow run."""
        self.jobs = await get_workflow_jobs(self.workflow_run.repository, self.workflow_run.id)
        await self.populate_jobs_tree()

    async def populate_jobs_tree(self) -> None:
        """Populate the tree with jobs and steps."""
        tree = self.query_one(Tree)
        tree.clear()

        if not self.jobs:
            tree.root.add_leaf("No jobs found")
            return

        for job in self.jobs:
            status_icon = self._format_job_status(job)
            job_label = f"{status_icon} {job.name} ({job.status})"
            job_node = tree.root.add(job_label, data={"type": "job", "job": job})

            for step in job.steps:
                step_status_icon = self._format_step_status(step)
                step_label = f"{step_status_icon} {step.name} ({step.status})"
                job_node.add_leaf(step_label, data={"type": "step", "step": step, "job": job})

    def _format_job_status(self, job: WorkflowJob) -> str:
        """Format job status with colors and symbols."""
        if job.status == WorkflowJobStatus.COMPLETED:
            if job.conclusion == WorkflowJobConclusion.SUCCESS:
                return "[greenyellow]✓[/]"
            elif job.conclusion == WorkflowJobConclusion.FAILURE:
                return "[red]✗[/]"
            elif job.conclusion == WorkflowJobConclusion.CANCELLED:
                return "[yellow]○[/]"
            elif job.conclusion == WorkflowJobConclusion.SKIPPED:
                return "[dim]−[/]"
        elif job.status == WorkflowJobStatus.IN_PROGRESS:
            return "[cyan]⋯[/]"

        return "[dim]?[/]"

    def _format_step_status(self, step: WorkflowStep) -> str:
        """Format step status with colors and symbols."""
        if step.status == WorkflowStepStatus.COMPLETED:
            if step.conclusion == WorkflowStepConclusion.SUCCESS:
                return "[greenyellow]✓[/]"
            elif step.conclusion == WorkflowStepConclusion.FAILURE:
                return "[red]✗[/]"
            elif step.conclusion == WorkflowStepConclusion.CANCELLED:
                return "[yellow]○[/]"
            elif step.conclusion == WorkflowStepConclusion.SKIPPED:
                return "[dim]−[/]"
        elif step.status == WorkflowStepStatus.IN_PROGRESS:
            return "[cyan]⋯[/]"

        return "[dim]?[/]"

    def action_view_logs(self) -> None:
        """Handle action to view logs for the selected job/step."""
        tree = self.query_one(Tree)
        if tree.cursor_node and tree.cursor_node.data:
            data = tree.cursor_node.data
            if data["type"] == "job":
                self.view_job_logs(data["job"])
            elif data["type"] == "step":
                # For steps, view the parent job logs
                self.view_job_logs(data["job"])

    @work
    async def view_job_logs(self, job: WorkflowJob) -> None:
        """Fetch and display logs for a specific job."""
        # Update the header to show which job we're viewing
        logs_header = self.query_one("#logs_header", Label)
        logs_header.update(f"[b]Logs for Job: {job.name}[/]")

        logs_display = self.query_one("#logs_display", Markdown)
        await logs_display.update("Loading logs...")

        logs = await get_job_logs(self.workflow_run.repository, job.id)

        if logs:
            self.current_job_logs = logs
            # Truncate if too long for display
            if len(logs) > 50000:
                display_logs = logs[:50000] + "\n\n... (logs truncated, too long to display)"
            else:
                display_logs = logs
            await logs_display.update(f"```\n{display_logs}\n```")
        else:
            await logs_display.update("No logs available or failed to fetch logs.")

    def compose(self) -> ComposeResult:
        with ScrollableContainer():
            yield Label(f"[b]Jobs for Run #{self.workflow_run.run_number}[/]")
            tree = Tree("Jobs", id="jobs_tree")
            tree.show_root = False  # Hide the root node to show jobs directly
            yield tree
            yield Rule()
            yield Label("[b]Job Logs[/]", id="logs_header")
            yield Markdown("Select a job and press 'L' or 'Enter' to view logs.", id="logs_display")


class WorkflowRunLogsTabPane(TabPane):
    """Tab showing full aggregated logs for the workflow run."""

    DEFAULT_CSS = """
    WorkflowRunLogsTabPane {
        overflow-y: auto;
    }

    WorkflowRunLogsTabPane Markdown {
        height: 1fr;
        overflow-y: auto;
    }
    """

    def __init__(self, workflow_run: WorkflowRun) -> None:
        super().__init__("Logs", id="workflow_run_logs_pane")
        self.workflow_run = workflow_run

    def on_mount(self) -> None:
        """Load full logs when the pane is mounted."""
        self.load_full_logs()

    @work
    async def load_full_logs(self) -> None:
        """Load full aggregated logs by fetching logs for all jobs."""
        logs_display = self.query_one("#full_logs_display", Markdown)
        await logs_display.update("Loading full workflow logs...")

        # Get all jobs
        jobs = await get_workflow_jobs(self.workflow_run.repository, self.workflow_run.id)

        if not jobs:
            await logs_display.update("No jobs found for this workflow run.")
            return

        # Fetch logs for each job and aggregate
        all_logs = []
        for job in jobs:
            all_logs.append(f"\n{'=' * 80}\n")
            all_logs.append(f"Job: {job.name} (Status: {job.status}, Conclusion: {job.conclusion})\n")
            all_logs.append(f"{'=' * 80}\n\n")

            job_logs = await get_job_logs(self.workflow_run.repository, job.id)
            if job_logs:
                all_logs.append(job_logs)
            else:
                all_logs.append("(No logs available for this job)\n")
            all_logs.append("\n")

        combined_logs = "".join(all_logs)

        # Truncate if too long
        if len(combined_logs) > 100000:
            display_logs = combined_logs[:100000] + "\n\n... (logs truncated, too long to display)"
        else:
            display_logs = combined_logs

        await logs_display.update(f"```\n{display_logs}\n```")

    def compose(self) -> ComposeResult:
        with ScrollableContainer():
            yield Label(f"[b]Full Logs for Run #{self.workflow_run.run_number}[/]")
            yield Markdown("Loading...", id="full_logs_display")
