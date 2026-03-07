{{/*
Expand the name of the chart.
*/}}
{{- define "weather-wms.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "weather-wms.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "weather-wms.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels applied to all resources.
*/}}
{{- define "weather-wms.labels" -}}
helm.sh/chart: {{ include "weather-wms.chart" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/part-of: weather-wms
{{ include "weather-wms.selectorLabels" . }}
{{- end }}

{{/*
Selector labels (used in matchLabels and pod templates).
*/}}
{{- define "weather-wms.selectorLabels" -}}
app.kubernetes.io/name: {{ include "weather-wms.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Construct a full image reference from global registry + repository + tag.
Usage: {{ include "weather-wms.image" (dict "registry" .Values.global.image.registry "repository" .repo "tag" .tag) }}
*/}}
{{- define "weather-wms.image" -}}
{{- if .registry }}
{{- printf "%s/%s:%s" .registry .repository .tag }}
{{- else }}
{{- printf "%s:%s" .repository .tag }}
{{- end }}
{{- end }}

{{/*
DATABASE_URL constructed from secret values.
*/}}
{{- define "weather-wms.databaseUrl" -}}
postgresql://{{ .Values.secrets.postgres.user }}:{{ .Values.secrets.postgres.password }}@{{ include "weather-wms.fullname" . }}-postgres:5432/{{ .Values.secrets.postgres.database }}
{{- end }}

{{/*
REDIS_URL constructed from secret values.
When redis password is empty, connect without auth.
*/}}
{{- define "weather-wms.redisUrl" -}}
{{- if .Values.secrets.redis.password -}}
redis://:{{ .Values.secrets.redis.password }}@{{ include "weather-wms.fullname" . }}-redis:6379
{{- else -}}
redis://{{ include "weather-wms.fullname" . }}-redis:6379
{{- end -}}
{{- end }}

{{/*
S3 endpoint URL (internal to cluster).
*/}}
{{- define "weather-wms.s3Endpoint" -}}
http://{{ include "weather-wms.fullname" . }}-minio:9000
{{- end }}

{{/*
EDR base URL.
*/}}
{{- define "weather-wms.edrBaseUrl" -}}
{{- if .Values.ingress.tls.enabled }}
https://{{ .Values.global.domain }}/edr
{{- else }}
http://{{ .Values.global.domain }}/edr
{{- end }}
{{- end }}
