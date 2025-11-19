from lazy_github.lib.context import LazyGithubContext, github_headers
from lazy_github.lib.github.backends.protocol import GithubApiRequestFailed
from lazy_github.lib.logging import lg
from lazy_github.models.github import Repository, Workflow, WorkflowJob, WorkflowRun


async def list_workflows(repository: Repository, page: int = 1, per_page: int = 30) -> list[Workflow]:
    """Lists available Github action workflows on the specified repo"""
    query_params = {"page": page, "per_page": per_page}
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/workflows"
    try:
        response = await LazyGithubContext.client.get(url, params=query_params)
        response.raise_for_status()
        raw_json = response.json()
    except GithubApiRequestFailed:
        lg.exception("Error retrieving actions from the Github API")
        return []
    else:
        if workflows := raw_json.get("workflows"):
            return [Workflow(**w) for w in workflows]
        else:
            return []


async def list_workflow_runs(repository: Repository, page: int = 1, per_page: int = 30) -> list[WorkflowRun]:
    """Lists github workflows runs on the specified repo"""
    query_params = {"page": page, "per_page": per_page}
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/runs"

    try:
        response = await LazyGithubContext.client.get(url, params=query_params)
        response.raise_for_status()
        raw_json = response.json()
    except GithubApiRequestFailed:
        lg.exception("Error retrieving action runs from the Github API")
        return []
    else:
        if workflows := raw_json.get("workflow_runs"):
            return [WorkflowRun(**w) for w in workflows]
        else:
            return []


async def create_dispatch_event(repository: Repository, workflow: Workflow, branch: str) -> bool:
    """
    Creates a workflow dispatch event for the specified workflow. For properly configured workflows, this will trigger
    a new one against the specified branch
    """
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/workflows/{workflow.id}/dispatches"
    body = {"ref": branch}
    response = await LazyGithubContext.client.post(url, headers=github_headers(), json=body)
    try:
        response.raise_for_status()
    except GithubApiRequestFailed:
        lg.exception("Error creating workflow dispatch event!")
    if not response.is_success:
        lg.error(f"Error creating workflow dispatch event: {response}")
    return response.is_success


async def get_workflow_run(repository: Repository, run_id: int) -> WorkflowRun | None:
    """Gets detailed information about a specific workflow run"""
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/runs/{run_id}"
    try:
        response = await LazyGithubContext.client.get(url)
        response.raise_for_status()
        return WorkflowRun(**response.json())
    except GithubApiRequestFailed:
        lg.exception(f"Error retrieving workflow run {run_id} from the Github API")
        return None


async def get_workflow_jobs(repository: Repository, run_id: int) -> list[WorkflowJob]:
    """Gets all jobs for a specific workflow run"""
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/runs/{run_id}/jobs"
    try:
        response = await LazyGithubContext.client.get(url)
        response.raise_for_status()
        raw_json = response.json()
        if jobs := raw_json.get("jobs"):
            return [WorkflowJob(**j) for j in jobs]
        else:
            return []
    except GithubApiRequestFailed:
        lg.exception(f"Error retrieving jobs for workflow run {run_id} from the Github API")
        return []


async def get_workflow_run_logs(repository: Repository, run_id: int) -> bytes | None:
    """
    Downloads the full logs for a workflow run as a zip file.
    Returns the raw bytes of the zip file, or None if the request fails.
    """
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/runs/{run_id}/logs"
    try:
        response = await LazyGithubContext.client.get(url)
        response.raise_for_status()
        return response.content
    except GithubApiRequestFailed:
        lg.exception(f"Error downloading logs for workflow run {run_id}")
        return None


async def get_job_logs(repository: Repository, job_id: int) -> str | None:
    """
    Gets the logs for a specific job.
    Returns the logs as plain text, or None if the request fails.
    """
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/jobs/{job_id}/logs"
    try:
        response = await LazyGithubContext.client.get(url)
        response.raise_for_status()
        return response.text
    except GithubApiRequestFailed:
        lg.exception(f"Error retrieving logs for job {job_id}")
        return None


async def rerun_workflow(repository: Repository, run_id: int) -> bool:
    """Re-runs an entire workflow run, including all jobs"""
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/runs/{run_id}/rerun"
    try:
        response = await LazyGithubContext.client.post(url, headers=github_headers())
        response.raise_for_status()
        return response.is_success
    except GithubApiRequestFailed:
        lg.exception(f"Error re-running workflow run {run_id}")
        return False


async def rerun_failed_jobs(repository: Repository, run_id: int) -> bool:
    """Re-runs only the failed jobs in a workflow run"""
    url = f"/repos/{repository.owner.login}/{repository.name}/actions/runs/{run_id}/rerun-failed-jobs"
    try:
        response = await LazyGithubContext.client.post(url, headers=github_headers())
        response.raise_for_status()
        return response.is_success
    except GithubApiRequestFailed:
        lg.exception(f"Error re-running failed jobs for workflow run {run_id}")
        return False
