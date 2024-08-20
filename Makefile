# usage:
#	make release tag=1.0.0

#In case a tag has been pushed to GitHub, but the release failed, run `
#	git tag --delete v1.0.0
#	git push --delete origin v1.0.0
# and repeat the steps below

tests:
	bash tests/tests.sh

release:
	bash tests/tests.sh
	mypy .
	black .
	git status
	grep -q $(tag) pyproject.toml || sed -i pyproject.toml -e "s|version = .*|version = \"$(tag)\"|"
	echo "Abort now if there are files that needs to be committed"
	sleep 10
	git tag v$(tag)
	# enter "v1.0.0"
	git-cliff -c pyproject.toml > CHANGELOG.md
	#toc -lf .tocfiles
	git tag --delete v$(tag)
	git add pyproject.toml || true
	git add CHANGELOG.md || true
	git commit -m "minor: updated CHANGELOG.md" || true
	git tag -fa v$(tag)
	git push --follow-tags
