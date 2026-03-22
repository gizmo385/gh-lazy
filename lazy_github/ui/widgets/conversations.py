from textual import work
from textual.app import ComposeResult
from textual.containers import Container
from textual.content import Content
from textual.widgets import Collapsible, Label, ListItem, ListView, Markdown, Static

from lazy_github.lib.bindings import LazyGithubBindings
from lazy_github.lib.github.pull_requests import ReviewCommentNode
from lazy_github.lib.messages import NewCommentCreated
from lazy_github.models.github import (
    FullPullRequest,
    Issue,
    IssueComment,
    ReactionSet,
    Review,
    ReviewComment,
    ReviewState,
)
from lazy_github.ui.screens.new_comment import NewCommentModal


class ReactionsDisplay(Container):
    DEFAULT_CSS = """
    ReactionsDisplay {
        height: auto;
    }
    Collapsible {
        height: auto;
    }

    ListView {
        height: auto;
    }
    """

    def __init__(self, item_id: str | int, id: str | None = None) -> None:
        super().__init__(id=id)
        self.item_id = item_id

    @property
    def reactions_list(self) -> ListView:
        return self.query_one(f"#reactions_list_{self.item_id}", ListView)

    @property
    def collapsible_reactions(self) -> Collapsible:
        return self.query_one(f"#collapsible_reactions_{self.item_id}", Collapsible)

    def compose(self) -> ComposeResult:
        with Collapsible(title="Reactions...", id=f"collapsible_reactions_{self.item_id}", collapsed=True):
            yield ListView(id=f"reactions_list_{self.item_id}")

    def on_mount(self) -> None:
        self.loading = True

    async def set_reactions(self, reactions: ReactionSet) -> None:
        self.loading = True
        await self.reactions_list.clear()
        summary_strings = [f"{rt.emoji} {count}" for rt, count in reactions.reaction_counts.items() if count]

        for reaction_type, users in reactions.users_by_reaction_type.items():
            if not users:
                continue
            elif len(users) > 3:
                users_string = f"{users[0].login}, {users[1].login}, {users[2].login}, and {len(users) - 3} more"
            else:
                users_string = ", ".join(u.login for u in users)

            reaction_label = f"{reaction_type.emoji}: {users_string}"
            self.reactions_list.append(ListItem(Label(Content.from_markup(reaction_label))))

        self.loading = False
        self.collapsible_reactions.title = " | ".join(summary_strings)
        self.collapsible_reactions.display = bool(summary_strings)


class IssueCommentContainer(Container, can_focus=True):
    DEFAULT_CSS = """
    IssueCommentContainer {
        height: auto;
        border-left: solid $secondary-background;
        margin-left: 1;
        margin-bottom: 1;
    }

    IssueCommentContainer:focus-within {
        border: dashed $success;
    }

    .comment-author {
        color: $text-muted;
        margin-left: 1;
        margin-bottom: 1;
    }

    Markdown {
        margin-left: 1;
        margin-bottom: 0;
        padding-bottom: 0;
    }

    """

    BINDINGS = [LazyGithubBindings.REPLY_TO_COMMENT]

    def __init__(self, issue: Issue, comment: IssueComment) -> None:
        super().__init__()
        self.issue = issue
        self.comment = comment

    def compose(self) -> ComposeResult:
        comment_time = self.comment.created_at.strftime("%c")
        author = self.comment.user.login if self.comment.user else "Unknown"
        yield Markdown(self.comment.body)
        yield Label(f"{author} • {comment_time}", classes="comment-author")

    @work
    async def reply_to_comment_flow(self) -> None:
        reply_comment = await self.app.push_screen_wait(NewCommentModal(self.issue.repo, self.issue, self.comment))
        if reply_comment is not None:
            self.post_message(NewCommentCreated(reply_comment))

    def action_reply_to_individual_comment(self) -> None:
        self.reply_to_comment_flow()

    async def add_reaction_display(self, reactions: ReactionSet) -> None:
        if not reactions:
            return

        rd = ReactionsDisplay(self.comment.id)
        await self.mount(rd, after=len(self.children) - 1)
        await rd.set_reactions(reactions)


class ReviewConversation(Container):
    DEFAULT_CSS = """
    ReviewConversation {
        height: auto;
        border-left: solid $secondary-background;
        margin-bottom: 1;
    }
    """

    def __init__(self, pr: FullPullRequest, root_conversation_node: ReviewCommentNode) -> None:
        super().__init__()
        self.pr = pr
        self.root_conversation_node = root_conversation_node

    def _flatten_comments(self, root: ReviewCommentNode) -> list[ReviewComment]:
        result = [root.comment]
        for child in root.children:
            result.extend(self._flatten_comments(child))
        return result

    def compose(self) -> ComposeResult:
        for comment in self._flatten_comments(self.root_conversation_node):
            yield IssueCommentContainer(self.pr, comment)


class ReviewContainer(Container):
    DEFAULT_CSS = """
    ReviewContainer {
        height: auto;
    }

    Collapsible {
        height: auto;
    }

    ReviewContainer:focus-within {
        border: solid $success-lighten-3;
    }
    """
    BINDINGS = [LazyGithubBindings.REPLY_TO_REVIEW]

    def __init__(self, pr: FullPullRequest, review: Review, hierarchy: dict[int, ReviewCommentNode]) -> None:
        super().__init__()
        self.pr = pr
        self.review = review
        self.hierarchy = hierarchy

    def compose(self) -> ComposeResult:
        if self.review.state == ReviewState.APPROVED:
            review_state_text = "[greenyellow]Approved[/]"
        elif self.review.state == ReviewState.CHANGES_REQUESTED:
            review_state_text = "[red]Changes Requested[/red]"
        else:
            review_state_text = self.review.state.title()
        review_summary = f"Review from {self.review.user.login} ({review_state_text})"

        if self.review.body or self.review.comments:
            with Collapsible(title=review_summary, collapsed=self.review.state == ReviewState.DISMISSED):
                if self.review.body:
                    yield Markdown(self.review.body)

                for comment in self.review.comments:
                    if comment_node := self.hierarchy.get(comment.id):
                        yield ReviewConversation(self.pr, comment_node)
        else:
            with Collapsible(title=review_summary, collapsed=True):
                yield Static("No additional review content")

    def action_reply_to_review(self) -> None:
        self.app.push_screen(NewCommentModal(self.pr.repo, self.pr, self.review))
