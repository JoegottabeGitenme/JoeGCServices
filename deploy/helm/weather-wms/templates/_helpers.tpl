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
Common labels
*/}}
{{- define "weather-wms.labels" -}}
helm.sh/chart: {{ include "weather-wms.chart" . }}
{{ include "weather-wms.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "weather-wms.selectorLabels" -}}
app.kubernetes.io/name: {{ include "weather-wms.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "weather-wms.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "weather-wms.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Get Redis URL
*/}}
{{- define "weather-wms.redisUrl" -}}
{{- if .Values.config.redis.url }}
{{- .Values.config.redis.url }}
{{- else if .Values.redis.enabled }}
{{- printf "redis://%s-redis-master:6379" .Release.Name }}
{{- else }}
{{- "redis://redis:6379" }}
{{- end }}
{{- end }}

{{/*
Get PostgreSQL URL
*/}}
{{- define "weather-wms.databaseUrl" -}}
{{- if .Values.config.database.url }}
{{- .Values.config.database.url }}
{{- else if .Values.postgresql.enabled }}
{{- printf "postgresql://%s:%s@%s-postgresql:5432/%s" .Values.postgresql.auth.username .Values.postgresql.auth.password .Release.Name .Values.postgresql.auth.database }}
{{- else }}
{{- "postgresql://postgres:postgres@postgres:5432/weatherwms" }}
{{- end }}
{{- end }}

{{/*
Get S3 endpoint
*/}}
{{- define "weather-wms.s3Endpoint" -}}
{{- if .Values.config.s3.endpoint }}
{{- .Values.config.s3.endpoint }}
{{- else if .Values.minio.enabled }}
{{- printf "http://%s-minio:9000" .Release.Name }}
{{- else }}
{{- "http://minio:9000" }}
{{- end }}
{{- end }}

{{/*
Get MinIO credentials
*/}}
{{- define "weather-wms.s3AccessKey" -}}
{{- if .Values.minio.enabled }}
{{- .Values.minio.auth.rootUser }}
{{- else }}
{{- "minioadmin" }}
{{- end }}
{{- end }}

{{- define "weather-wms.s3SecretKey" -}}
{{- if .Values.minio.enabled }}
{{- .Values.minio.auth.rootPassword }}
{{- else }}
{{- "minioadmin" }}
{{- end }}
{{- end }}
