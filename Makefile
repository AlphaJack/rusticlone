# usage:
#	make release tag=1.0.0

#In case a tag has been pushed to GitHub, but the release failed, run `
#	git tag --delete v1.0.0
#	git push --delete origin v1.0.0
# and repeat the steps below

install:
	uv sync --all-groups

lint:
	ty check .
	ruff check --fix .
	ruff format .

test:
	uv run bash tests/tests.sh

ci:
	act --workflows ".github/workflows/test.yml"

toc:
	find * -type f ! -name 'CHANGELOG.md' -exec toc -f {} \; 2>/dev/null

review:
	git status
	echo "Abort now if there are files that needs to be committed"
	sleep 10

tag_bump:
	grep -q $(tag) pyproject.toml || sed -i pyproject.toml -e "s|version = .*|version = \"$(tag)\"|"

tag_changelog:
	git tag v$(tag) -m v$(tag)
	# enter "v1.0.0"
	git-cliff -c pyproject.toml > CHANGELOG.md

tag_commit_new_changelog:
	git tag --delete v$(tag)
	git add --all || true
	git commit -m "minor: release $(tag)" || true
	git tag -fa v$(tag) -m v$(tag)

tag_publish::
	git push --follow-tags

tag: tag_bump tag_changelog tag_commit_new_changelog tag_publish

release: lint toc review tag
