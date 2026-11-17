//! A qualified release being fetched for review, and a connected radio's firmware standing.

use signalman::comms_adapter::comms::ProtocolKind;
use signalman::{FirmwareDownloadWorker, FirmwareStanding, RadioFirmware};

use super::DesktopState;

impl DesktopState {
    /// Adopt a download Signalman started for the package being reviewed.
    pub fn begin_firmware_download(&mut self, worker: FirmwareDownloadWorker, note: String) {
        self.firmware_download = Some(worker);
        self.firmware_download_note = Some(note);
        self.refusal.clear();
    }

    /// Whether a download has ended and is waiting for the UI thread. Never blocks.
    pub fn firmware_download_finished(&self) -> bool {
        self.firmware_download
            .as_ref()
            .is_some_and(FirmwareDownloadWorker::finished)
    }

    /// Hand a finished, verified download to the owner flow, whose review page is the one
    /// confirmation; a refused download becomes visible text.
    pub fn finish_firmware_download(&mut self) {
        let Some(result) = self.firmware_download.as_mut().and_then(|w| w.take()) else {
            return;
        };
        self.firmware_download = None;
        self.firmware_download_note = None;
        match result {
            Ok(package) => match self.installer.choose_firmware(package) {
                Ok(()) => self.refusal.clear(),
                Err(error) => self.refuse(&error),
            },
            Err(error) => self.refuse_with(vec![error]),
        }
    }

    /// Where the connected radio's firmware stands against the catalog. MeshCore reports no
    /// board this build maps, so the owner's board choice applies when there is one.
    pub fn companion_standing(&self, protocol: ProtocolKind) -> Option<FirmwareStanding> {
        let firmware = self.companion_firmware(protocol)?;
        Some(match &self.catalog {
            Some(catalog) => catalog.standing(Some(firmware), self.selected_board_family.clone()),
            None => FirmwareStanding::Unknown {
                reason: "no verified package catalog is loaded".into(),
            },
        })
    }

    pub fn companion_firmware(&self, protocol: ProtocolKind) -> Option<&RadioFirmware> {
        self.companion_firmware
            .iter()
            .find(|(kind, _)| *kind == protocol)
            .map(|(_, firmware)| firmware)
    }

    /// Select a listed package by id, for the owner to review on the firmware pages.
    pub fn choose_package_id(&mut self, package_id: &str) {
        let index = self.catalog.as_ref().and_then(|catalog| {
            catalog
                .packages()
                .iter()
                .position(|package| package.package_id == package_id)
        });
        if let Some(index) = index {
            self.select_package(index);
            self.message_notice = Some(format!(
                "{package_id} is chosen. Disconnect the radio, put it in its bootloader, and review it on the Devices pages."
            ));
        }
    }
}
