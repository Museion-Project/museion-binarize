"""Evidence validation for the user-scoped macOS-only, OCR-disabled candidate.

Legacy cross-platform/source profiles remain unchanged. No report can infer
notarization from signing, or pass a missing/altered artifact or evidence file.
"""
import hashlib,json,re
from pathlib import Path
from sign_local_macos import bundle_digest

CHECKS={
 'traceable_build':{'committed_source','source_input_hashes','release_build','ocr_disabled_invoke_surface'},
 'portable_runtime':{'arm64_macho_closure','minimum_os_audit','no_external_python_or_native_libraries','frozen_dispatch_save','precompiled_vision','sdk27_image_branch','dependency_notices','runtime_sbom'},
 'functional_tests':{'frontend','local_pdf_contracts','hierarchy_contracts','rust_desktop','distribution_contracts','qpdf_strict'},
 'signed_install_workflow':{'developer_id_deep_strict','hardened_runtime','dmg_mount_copy_unmount','relocated_launch','native_picker_open','binarize_save_reopen','bookmark_edit_save','review_revoked_after_edit','locale_document_preserved','clean_exit'},
 'reader_matrix':{'pdfium_pixels_text_destinations','pypdf_outline_destinations','pdfkit_outline_destinations','qpdf_strict'},
 'upgrade_install':{'bundle_identity_preserved','candidate_replacement','locale_preference_preserved','document_open_after_replacement'},
 'privacy_accessibility_performance':{'no_cloud_or_body_ocr_command','local_processing_no_network_observed','native_controls_accessible','keyboard_open_navigation_save','processing_timings_recorded'},
 'developer_id_notary':{'apple_accepted','dmg_stapled','stapler_validated','apple_distribution_policy'},
}
REQUIRED=set(CHECKS)|{'macos_release_artifacts'}


def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def evaluate(path):
    gates={key:'pending' for key in REQUIRED}
    if path is None:return gates
    try:
        path=Path(path);evidence=json.loads(path.read_text())
        if evidence['schema']!='mpdf-macos-local-evidence/1' or evidence['target']!='aarch64-apple-darwin':raise ValueError('wrong evidence schema/target')
        if not re.fullmatch(r'[0-9a-f]{40}',evidence['source_commit']):raise ValueError('missing source commit')
        app=Path(evidence['app']['path']);app_hash=evidence['app']['bundle_sha256']
        if not app.is_dir() or bundle_digest(app)!=app_hash:raise ValueError('app digest mismatch')
        dmg=Path(evidence['dmg']['path']);dmg_hash=evidence['dmg']['sha256']
        if not dmg.is_file() or digest(dmg)!=dmg_hash:raise ValueError('DMG digest mismatch')
        gates['macos_release_artifacts']='pass_local'
    except (OSError,KeyError,ValueError,TypeError):
        gates['macos_release_artifacts']='fail';return gates
    for gate,required in CHECKS.items():
        try:
            record=evidence['records'][gate];record_path=path.parent/record['path']
            if not record_path.resolve().is_relative_to(path.parent.resolve()):raise ValueError('evidence path escapes report directory')
            if digest(record_path)!=record['sha256']:raise ValueError('evidence digest mismatch')
            data=json.loads(record_path.read_text())
            if data['source_commit']!=evidence['source_commit'] or data['artifact_bundle_sha256']!=app_hash:raise ValueError('evidence belongs to another source/artifact')
            checks=data['checks']
            if data.get('status')!='pass' or not required.issubset(checks) or any(checks[key]!='pass' for key in required):continue
            if gate=='developer_id_notary':
                if data['dmg_sha256']!=dmg_hash or data.get('notarization_state')!='accepted_stapled' or not data.get('submission_id'):continue
                if data.get('policy_method')!='syspolicy_check distribution' or data.get('policy_exit_code')!=0:continue
            if not data.get('evidence_files'):continue
            for attachment in data['evidence_files']:
                file=path.parent/attachment['path']
                if not file.resolve().is_relative_to(path.parent.resolve()) or digest(file)!=attachment['sha256']:raise ValueError('evidence attachment mismatch')
            gates[gate]='pass_local'
        except KeyError:continue
        except (OSError,ValueError,TypeError):gates[gate]='fail'
    return gates
