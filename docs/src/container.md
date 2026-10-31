# Container image

To use the container image, mount your rustic profiles, Rusticlone's state, the files you want to back up and their backup destination.
Set `TZ` to your time zone, so that schedules such as `@daily` follow your clock rather than UTC.

For the `Documents` collection above:

```sh
mkdir -p rustic/{profiles,state}
docker run \
  -e RUSTICLONE_ADMIN_TOKEN=XXXXXX \
  -e TZ=Europe/Berlin \
  -p 32110:32110 \
  -v $PWD/rustic/profiles:/etc/rustic \
  -v $PWD/rustic/state:/var/lib/rusticlone \
  -v /home/jack/Documents:/home/jack/Documents \
  -v /mnt/hdd/backups:/mnt/hdd/backups \
  ghcr.io/alphajack/rusticlone:latest
```

You can then access the web UI from port 32110.
